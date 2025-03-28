use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;

use crate::{get_paths, struct_fields, EnumField, TypeSet};

pub fn derive_diff(input: TokenStream) -> syn::Result<TokenStream2> {
    let input: syn::DeriveInput = syn::parse(input)?;
    let identifier = &input.ident;
    let (firewheel_path, diff_path) = get_paths();

    let (impl_generics, ty_generics, where_generics) = input.generics.split_for_impl();

    fn generate_where(
        where_clause: Option<&syn::WhereClause>,
        bounds: impl Iterator<Item = TokenStream2>,
    ) -> TokenStream2 {
        match where_clause {
            Some(wg) => {
                quote! {
                    #wg
                    #(#bounds,)*
                }
            }
            None => {
                quote! {
                    where #(#bounds,)*
                }
            }
        }
    }

    let (body, where_generics) = match &input.data {
        syn::Data::Struct(data) => {
            let DiffOutput { body, bounds } = DiffOutput::from_struct(data, &diff_path)?;

            (body, generate_where(where_generics, bounds))
        }
        syn::Data::Enum(data) => {
            let DiffOutput { body, bounds } =
                DiffOutput::from_enum(identifier, data, &firewheel_path, &diff_path)?;

            (body, generate_where(where_generics, bounds))
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new(
                input.span(),
                "`Diff` cannot be derived on unions.",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics #diff_path::Diff for #identifier #ty_generics #where_generics {
            fn diff<__E: #diff_path::EventQueue>(&self, baseline: &Self, path: #diff_path::PathBuilder, event_queue: &mut __E) {
                #body
            }
        }
    })
}

struct DiffOutput<B> {
    body: TokenStream2,
    bounds: B,
}

impl DiffOutput<()> {
    pub fn from_struct<'a>(
        data: &'a syn::DataStruct,
        diff_path: &'a TokenStream2,
    ) -> syn::Result<DiffOutput<impl Iterator<Item = TokenStream2> + use<'a>>> {
        let fields: Vec<_> = struct_fields(&data.fields).collect();

        let arms = fields.iter().enumerate().map(|(i, (identifier, _))| {
            let index = i as u32;
            quote! {
                self.#identifier.diff(&baseline.#identifier, path.with(#index), event_queue);
            }
        });

        let mut types = TypeSet::default();
        for field in &fields {
            types.insert(field.1);
        }

        Ok(DiffOutput {
            body: quote! { #(#arms)* },
            bounds: types.into_iter().map(move |ty| {
                let span = ty.span();
                quote_spanned! {span=> #ty: #diff_path::Diff }
            }),
        })
    }

    // This is a fair bit more complicated because we need to account for
    // three kinds of variants _and_ we need to be able to construct variants
    // with all required data at once in addition to fine-grained diffing.
    pub fn from_enum<'a>(
        identifier: &'a syn::Ident,
        data: &'a syn::DataEnum,
        firewheel_path: &'a syn::Path,
        diff_path: &'a TokenStream2,
    ) -> syn::Result<DiffOutput<impl Iterator<Item = TokenStream2> + use<'a>>> {
        let mut arms = Vec::new();
        let mut types = TypeSet::default();
        for (index, variant) in data.variants.iter().enumerate() {
            let variant_index = index as u32;
            let variant_ident = &variant.ident;

            match &variant.fields {
                syn::Fields::Unit => {
                    arms.push(quote! {
                        (#identifier::#variant_ident, #identifier::#variant_ident) => {}
                    });

                    arms.push(quote! {
                        (#identifier::#variant_ident, _) => {
                            event_queue.push_param(
                                #firewheel_path::event::ParamData::U32(0),
                                path.with(#variant_index),
                            );
                        }
                    });
                }
                fields => {
                    let filtered_fields = fields
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| !crate::should_skip(&f.attrs));

                    let mut a_idents = Vec::new();
                    let mut b_idents = Vec::new();

                    for (i, field) in filtered_fields {
                        types.insert(&field.ty);

                        a_idents.push(EnumField {
                            ty: &field.ty,
                            unpack_ident: format_ident!("a{i}"),
                            member: crate::as_member(field.ident.as_ref(), i),
                        });

                        b_idents.push(EnumField {
                            ty: &field.ty,
                            unpack_ident: format_ident!("b{i}"),
                            member: crate::as_member(field.ident.as_ref(), i),
                        });
                    }

                    let diff_statements = a_idents.iter().zip(b_idents.iter()).enumerate().map(
                        |(i, (a, b))| {
                            let i = i as u32;
                            let ty = &a.ty;
                            let a = &a.unpack_ident;
                            let b = &b.unpack_ident;

                            quote! {
                                <#ty as #diff_path::Diff>::diff(#a, #b, path.with(#i), event_queue);
                            }
                        },
                    );

                    let a_unpacked: Vec<_> = a_idents.iter().map(EnumField::unpack).collect();
                    let b_unpacked = b_idents.iter().map(EnumField::unpack);

                    let inner = quote! {
                        (#identifier::#variant_ident{#(#a_unpacked),*}, #identifier::#variant_ident{#(#b_unpacked),*}) => {
                            let path = path.with(#variant_index);

                            #(#diff_statements)*
                        }
                    };

                    arms.push(inner);

                    let set_items = a_idents.iter().map(|a| {
                        let ty = &a.ty;
                        let a = &a.unpack_ident;

                        quote! { <#ty as ::core::clone::Clone>::clone(&#a) }
                    });

                    let outer = quote! {
                        (#identifier::#variant_ident{#(#a_unpacked),*}, _) => {
                            event_queue.push_param(
                                #firewheel_path::event::ParamData::any((#(#set_items,)*)),
                                path.with(#variant_index),
                            );
                        }
                    };

                    arms.push(outer);
                }
            }
        }

        let body = quote! {
            match (self, baseline) {
                #(#arms)*
            }
        };

        Ok(DiffOutput {
            body,
            bounds: types.into_iter().map(move |ty| {
                let span = ty.span();
                quote_spanned! {span=>
                    #ty: #diff_path::Diff
                        + ::core::clone::Clone
                        + ::core::marker::Send
                        + ::core::marker::Sync
                        + 'static
                }
            }),
        })
    }
}
