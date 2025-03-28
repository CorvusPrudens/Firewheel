use bevy_macro_utils::fq_std::FQResult;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;

use crate::{get_paths, struct_fields, EnumField, TypeSet};

pub fn derive_patch(input: TokenStream) -> syn::Result<TokenStream2> {
    let input: syn::DeriveInput = syn::parse(input)?;
    let identifier = &input.ident;
    let (firewheel_path, diff_path) = get_paths();

    let patch_ident = format_ident!("{identifier}Patch");
    let vis = &input.vis;

    let PatchOutput {
        create_update_struct,
        patch_body,
        apply_body,
        bounds,
        fields,
    } = match &input.data {
        syn::Data::Struct(data) => PatchOutput::from_struct(data, &diff_path, &patch_ident)?,
        syn::Data::Enum(data) => {
            PatchOutput::from_enum(identifier, data, &diff_path, &patch_ident)?
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new(
                input.span(),
                "`Patch` cannot be derived on unions.",
            ));
        }
    };

    let (impl_generics, ty_generics, where_generics) = input.generics.split_for_impl();

    let where_generics = match where_generics {
        Some(wg) => {
            quote! {
                #wg
                #(#bounds,)*
            }
        }
        None => {
            if bounds.is_empty() {
                quote! {}
            } else {
                quote! {
                    where #(#bounds,)*
                }
            }
        }
    };

    let update_struct = create_update_struct.then(|| {
        quote! {
            #vis enum #patch_ident #impl_generics #where_generics {
                #(#fields),*
            }
        }
    });

    let patch = if create_update_struct {
        quote! {
            #patch_ident #ty_generics
        }
    } else {
        quote! { Self }
    };

    Ok(quote! {
        #update_struct

        impl #impl_generics #diff_path::Patch for #identifier #ty_generics #where_generics {
            type Patch = #patch;

            fn patch(
                data: &#firewheel_path::event::ParamData,
                path: &[u32]
            ) -> #FQResult<Self::Patch, #diff_path::PatchError> {
                #patch_body
            }

            fn apply(
                &mut self,
                patch: Self::Patch,
            ) {
                #apply_body
            }
        }
    })
}

struct PatchOutput {
    create_update_struct: bool,
    patch_body: TokenStream2,
    apply_body: TokenStream2,
    fields: Vec<TokenStream2>,
    bounds: Vec<TokenStream2>,
}

fn snake_to_camel(ident: &syn::Ident) -> syn::Ident {
    let ident_string = ident.to_string();

    let mut to_caps = true;
    let mut output = String::with_capacity(ident_string.len());

    for char in ident_string.chars() {
        if char == '_' {
            to_caps = true;
            continue;
        }

        if to_caps {
            to_caps = false;
            let char = char.to_ascii_uppercase();
            output.push(char);
        }
    }

    format_ident!("{output}")
}

impl PatchOutput {
    pub fn from_struct(
        data: &syn::DataStruct,
        diff_path: &TokenStream2,
        patch_ident: &syn::Ident,
    ) -> syn::Result<Self> {
        let fields: Vec<_> = struct_fields(&data.fields).collect();

        let patch_field_names: Vec<_> = fields
            .iter()
            .map(|f| match &f.0 {
                syn::Member::Named(name) => snake_to_camel(name),
                syn::Member::Unnamed(index) => format_ident!("Field{}", index.index),
            })
            .collect();

        let patch_fields = fields
            .iter()
            .zip(&patch_field_names)
            .map(|((_, ty), name)| {
                quote! {
                    #name(<#ty as #diff_path::Patch>::Patch)
                }
            });

        let patch_arms = fields.iter().zip(&patch_field_names).enumerate().map(|(i, ((_, ty), name))| {
            let index = i as u32;
            quote! {
                [#index, tail @ .. ] => Ok(#patch_ident::#name(<#ty as #diff_path::Patch>::patch(data, tail)?))
            }
        });

        let patch_body = quote! {
            match path {
                #(#patch_arms,)*
                _ => #FQResult::Err(#diff_path::PatchError::InvalidPath),
            }
        };

        let apply_arms = fields.iter().zip(&patch_field_names).map(|((member, ty), variant)| {
            quote! {
                #patch_ident::#variant(p) => <#ty as #diff_path::Patch>::apply(&mut self.#member, p)
            }
        });

        let apply_body = quote! {
            match patch {
                #(#apply_arms,)*
            }
        };

        let mut types = TypeSet::default();
        for field in &fields {
            types.insert(field.1);
        }

        Ok(Self {
            create_update_struct: true,
            apply_body,
            patch_body,
            fields: patch_fields.collect(),
            bounds: types
                .iter()
                .map(|ty| {
                    let span = ty.span();
                    quote_spanned! {span=> #ty: #diff_path::Patch }
                })
                .collect(),
        })
    }

    pub fn from_enum(
        identifier: &syn::Ident,
        data: &syn::DataEnum,
        diff_path: &TokenStream2,
        patch_ident: &syn::Ident,
    ) -> syn::Result<PatchOutput> {
        if data.variants.iter().all(|v| v.fields.is_empty()) {
            // trivial unit enum
            let patch_arms = data.variants.iter().enumerate().map(|(i, variant)| {
                let index = i as u32;
                let ident = &variant.ident;
                quote! {
                    [#index] => Ok(Self::#ident)
                }
            });

            let patch_body = quote! {
                match path {
                    #(#patch_arms,)*
                    _ => #FQResult::Err(#diff_path::PatchError::InvalidPath),
                }
            };

            let apply_body = quote! {
                *self = patch;
            };

            return Ok(Self {
                create_update_struct: false,
                patch_body,
                apply_body,
                fields: Vec::new(),
                bounds: Vec::new(),
            });
        }

        let mut patch_variants = vec![quote! {
            Variant(#identifier)
        }];
        let mut arms = Vec::new();
        let mut types = TypeSet::default();
        for (index, variant) in data.variants.iter().enumerate() {
            let variant_index = index as u32;
            let variant_ident = &variant.ident;

            match &variant.fields {
                syn::Fields::Unit => {
                    arms.push(quote! {
                        ([#variant_index], s) => {
                            *s = #identifier::#variant_ident;

                            Ok(())
                        }
                    });
                }
                fields => {
                    let fields: Vec<_> = fields
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| !crate::should_skip(&f.attrs))
                        .map(|(i, f)| {
                            let ty = &f.ty;
                            types.insert(ty);

                            let member = crate::as_member(f.ident.as_ref(), i);

                            let inner_ident = match member {
                                syn::Member::Named(name) => {
                                    format_ident!("{variant_ident}{}", snake_to_camel(&name))
                                }
                                syn::Member::Unnamed(_) => format_ident!("{variant_ident}{i}"),
                            };

                            patch_variants.push(quote! {
                                #inner_ident(<#ty as #diff_path::Patch>::Patch)
                            });

                            (
                                EnumField {
                                    member: crate::as_member(f.ident.as_ref(), i),
                                    ty,
                                    unpack_ident: format_ident!("d{i}"),
                                },
                                inner_ident,
                            )
                        })
                        .collect();

                    let unpacked = fields.iter().map(|i| &i.0.unpack_ident);
                    let unpacked_types = fields.iter().map(|i| i.0.ty);

                    let structured = fields.iter().map(|(i, _)| {
                        let member = &i.member;
                        let unpack = &i.unpack_ident;
                        let ty = i.ty;
                        quote! { #member: <#ty as ::core::clone::Clone>::clone(#unpack) }
                    });

                    let outer = quote! {
                        ([#variant_index], s) => {
                            let (#(#unpacked,)*): &(#(#unpacked_types,)*) = data
                                .downcast_ref()
                                .ok_or(#diff_path::PatchError::InvalidData)?;

                            *s = #identifier::#variant_ident{ #(#structured),* };
                            Ok(())
                        }
                    };

                    arms.push(outer);

                    let destructured: Vec<_> = fields
                        .iter()
                        .map(|(i, _)| {
                            let member = &i.member;
                            let unpack = &i.unpack_ident;
                            quote! { #member: #unpack }
                        })
                        .collect();

                    let inner = fields.iter().enumerate().map(|(i, (a, _))| {
                        let ty = &a.ty;
                        let a = &a.unpack_ident;
                        let i = i as u32;

                        quote! {
                            ([#variant_index, #i, tail @ ..], #identifier::#variant_ident{ #(#destructured),* }) => {
                                <#ty as #diff_path::Patch>::patch(#a, data, tail)
                            }
                        }
                    });

                    arms.extend(inner);
                }
            }
        }

        let body = quote! {
            match (path, self) {
                #(#arms)*
                _ => #FQResult::Err(#diff_path::PatchError::InvalidPath),
            }
        };

        Ok(Self {
            create_update_struct: true,
            fields: patch_variants,
            patch_body: quote! {},
            apply_body: quote! {},
            bounds: types
                .iter()
                .map(|ty| {
                    let span = ty.span();
                    quote_spanned! {span=>
                        #ty: #diff_path::Patch
                            + ::core::clone::Clone
                            + ::core::marker::Send
                            + ::core::marker::Sync
                            + 'static
                    }
                })
                .collect(),
        })
    }
}
