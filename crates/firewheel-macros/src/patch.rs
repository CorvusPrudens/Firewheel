use bevy_macro_utils::fq_std::FQResult;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;

use crate::{get_paths, struct_fields};

pub fn derive_patch(input: TokenStream) -> syn::Result<TokenStream2> {
    let input: syn::DeriveInput = syn::parse(input)?;
    let identifier = &input.ident;
    let (firewheel_path, diff_path) = get_paths();

    let PatchOutput { body, bounds } = match &input.data {
        syn::Data::Struct(data) => PatchOutput::from_struct(data, &diff_path)?,
        syn::Data::Enum(data) => {
            PatchOutput::from_enum(identifier, data, &firewheel_path, &diff_path)?
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

    Ok(quote! {
        impl #impl_generics #diff_path::Patch for #identifier #ty_generics #where_generics {
            fn patch(
                &mut self,
                data: &#firewheel_path::event::ParamData,
                path: &[u32]
            ) -> #FQResult<(), #diff_path::PatchError> {
                #body
            }
        }
    })
}

struct PatchOutput {
    body: TokenStream2,
    bounds: Vec<TokenStream2>,
}

struct UnnamedField<'a> {
    /// The type is useful for error spans.
    ty: &'a syn::Type,
    /// An identifier for tuple fields.
    unpack_ident: syn::Ident,
}

/// A convenience struct for keeping track of a struct variant's
/// identifier along with an identifier we can use without causing
/// name clashing.
struct NamedField<'a> {
    /// The type is useful for error spans.
    ty: &'a syn::Type,
    /// The struct field's actual name.
    type_ident: &'a syn::Ident,
    /// An identifier that avoids the possibility of name clashing.
    unpack_ident: syn::Ident,
}

impl PatchOutput {
    pub fn from_struct(data: &syn::DataStruct, diff_path: &TokenStream2) -> syn::Result<Self> {
        let fields = struct_fields(data);

        let arms = fields.iter().enumerate().map(|(i, (identifier, _))| {
            let index = i as u32;
            quote! {
                [#index, tail @ .. ] => self.#identifier.patch(data, tail)
            }
        });

        let body = quote! {
            match path {
                #(#arms,)*
                _ => #FQResult::Err(#diff_path::PatchError::InvalidPath),
            }
        };

        Ok(Self {
            body,
            bounds: fields
                .iter()
                .map(|(_, ty)| {
                    let span = ty.span();
                    quote_spanned! {span=> #ty: #diff_path::Patch }
                })
                .collect(),
        })
    }

    // This is quite a bit more complicated because we need to account for
    // three kinds of variants _and_ we need to be able to construct variants
    // with all required data at once in addition to fine-grained diffing.
    pub fn from_enum(
        identifier: &syn::Ident,
        data: &syn::DataEnum,
        firewheel_path: &syn::Path,
        diff_path: &TokenStream2,
    ) -> syn::Result<PatchOutput> {
        let mut arms = Vec::new();
        let mut types = Vec::new();
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
                syn::Fields::Unnamed(fields) => {
                    let mut idents = Vec::new();

                    for (i, field) in fields.unnamed.iter().enumerate() {
                        types.push(&field.ty);

                        idents.push(UnnamedField {
                            ty: &field.ty,
                            unpack_ident: format_ident!("a{i}"),
                        });
                    }

                    let unpacked: Vec<_> = idents
                        .iter()
                        .map(|a| {
                            let unpack = &a.unpack_ident;

                            quote! {
                                #unpack
                            }
                        })
                        .collect();

                    let unpacked_types = idents.iter().map(|i| i.ty);
                    let unpacked_cloned = idents.iter().map(|i| {
                        let unpack = &i.unpack_ident;
                        let ty = i.ty;
                        quote! { <#ty as ::core::clone::Clone>::clone(#unpack) }
                    });
                    arms.push(quote! {
                        ([#variant_index], s) => {
                            let (#(#unpacked,)*): &(#(#unpacked_types,)*) = data.downcast_ref().ok_or(PatchError::InvalidData)?;

                            *s = #identifier::#variant_ident(#(#unpacked_cloned),*);
                            Ok(())
                        }
                    });

                    let inner = idents.iter().enumerate().map(|(i, a)| {
                        let ty = &a.ty;
                        let a = &a.unpack_ident;
                        let i = i as u32;

                        quote! {
                            ([#variant_index, #i, tail @ ..], #identifier::#variant_ident(#(#unpacked),*)) => {
                                <#ty as #diff_path::Patch>::patch(#a, data, tail)
                            }
                        }
                    });

                    arms.extend(inner);
                }
                syn::Fields::Named(fields) => {
                    let mut idents = Vec::new();

                    for (i, field) in fields.named.iter().enumerate() {
                        types.push(&field.ty);

                        idents.push(NamedField {
                            ty: &field.ty,
                            type_ident: &field.ident.as_ref().expect("should have named ident"),
                            unpack_ident: format_ident!("a{i}"),
                        });
                    }

                    let tuple = idents.iter().map(|a| {
                        let unpack = &a.unpack_ident;

                        quote! {
                            #unpack
                        }
                    });

                    let unpacked_cloned = idents.iter().map(|a| {
                        let unpack = &a.unpack_ident;
                        let ident = &a.type_ident;
                        let ty = &a.ty;

                        quote! {
                            #ident: <#ty as ::core::clone::Clone>::clone(#unpack)
                        }
                    });

                    let unpacked_types = idents.iter().map(|i| i.ty);
                    arms.push(quote! {
                        ([#variant_index], s) => {
                            let (#(#tuple,)*): &(#(#unpacked_types,)*) = data.downcast_ref().ok_or(PatchError::InvalidData)?;

                            *s = #identifier::#variant_ident{#(#unpacked_cloned),*};
                            Ok(())
                        }
                    });

                    let unpacked: Vec<_> = idents
                        .iter()
                        .map(|a| {
                            let unpack = &a.unpack_ident;
                            let ident = &a.type_ident;

                            quote! {
                                #ident: #unpack
                            }
                        })
                        .collect();

                    let inner = idents.iter().enumerate().map(|(i, a)| {
                        let ty = &a.ty;
                        let a = &a.unpack_ident;
                        let i = i as u32;

                        quote! {
                            ([#variant_index, #i, tail @ ..], #identifier::#variant_ident{#(#unpacked),*}) => {
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
            body,
            bounds: types
                .iter()
                .map(|ty| {
                    let span = ty.span();
                    quote_spanned! {span=> #ty: #diff_path::Patch + ::core::clone::Clone + ::core::marker::Send + ::core::marker::Sync + 'static }
                })
                .collect(),
        })
    }
}
