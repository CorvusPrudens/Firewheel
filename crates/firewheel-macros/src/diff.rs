use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;

use crate::{get_paths, struct_fields};

pub fn derive_diff(input: TokenStream) -> syn::Result<TokenStream2> {
    let input: syn::DeriveInput = syn::parse(input)?;
    let identifier = &input.ident;
    let (firewheel_path, diff_path) = get_paths();

    let DiffOutput { body, bounds } = match &input.data {
        syn::Data::Struct(data) => DiffOutput::from_struct(data, &diff_path)?,
        syn::Data::Enum(data) => {
            DiffOutput::from_enum(identifier, data, &firewheel_path, &diff_path)?
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new(
                input.span(),
                "`Diff` cannot be derived on unions.",
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
        impl #impl_generics #diff_path::Diff for #identifier #ty_generics #where_generics {
            fn diff<__E: #diff_path::EventQueue>(&self, baseline: &Self, path: #diff_path::PathBuilder, event_queue: &mut __E) {
                #body
            }
        }
    })
}

struct DiffOutput {
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

impl DiffOutput {
    pub fn from_struct(data: &syn::DataStruct, diff_path: &TokenStream2) -> syn::Result<Self> {
        let fields = struct_fields(data);

        let arms = fields.iter().enumerate().map(|(i, (identifier, _))| {
            let index = i as u32;
            quote! {
                self.#identifier.diff(&baseline.#identifier, path.with(#index), event_queue);
            }
        });

        Ok(Self {
            body: quote! { #(#arms)* },
            bounds: fields
                .iter()
                .map(|(_, ty)| {
                    let span = ty.span();
                    quote_spanned! {span=> #ty: #diff_path::Diff }
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
    ) -> syn::Result<DiffOutput> {
        let mut arms = Vec::new();
        let mut types = Vec::new();
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
                syn::Fields::Unnamed(fields) => {
                    let mut a_idents = Vec::new();
                    let mut b_idents = Vec::new();

                    for (i, field) in fields.unnamed.iter().enumerate() {
                        types.push(&field.ty);

                        a_idents.push(UnnamedField {
                            ty: &field.ty,
                            unpack_ident: format_ident!("a{i}"),
                        });
                        b_idents.push(UnnamedField {
                            ty: &field.ty,
                            unpack_ident: format_ident!("b{i}"),
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

                    let a_unpacked: Vec<_> = a_idents
                        .iter()
                        .map(|a| {
                            let unpack = &a.unpack_ident;

                            quote! {
                                #unpack
                            }
                        })
                        .collect();

                    let b_unpacked = b_idents.iter().map(|b| {
                        let unpack = &b.unpack_ident;

                        quote! {
                            #unpack
                        }
                    });

                    arms.push(quote! {
                        (#identifier::#variant_ident(#(#a_unpacked),*), #identifier::#variant_ident(#(#b_unpacked),*)) => {
                            let path = path.with(#variant_index);

                            #(#diff_statements)*
                        }
                    });

                    let set_items = a_idents.iter().map(|a| {
                        let ty = &a.ty;
                        let a = &a.unpack_ident;

                        quote! { <#ty as ::core::clone::Clone>::clone(&#a) }
                    });

                    arms.push(quote! {
                        (#identifier::#variant_ident(#(#a_unpacked),*), _) => {
                            event_queue.push_param(
                                #firewheel_path::event::ParamData::any((#(#set_items,)*)),
                                path.with(#variant_index),
                            );
                        }
                    })
                }
                syn::Fields::Named(fields) => {
                    let mut a_idents = Vec::new();
                    let mut b_idents = Vec::new();

                    for (i, field) in fields.named.iter().enumerate() {
                        types.push(&field.ty);

                        a_idents.push(NamedField {
                            ty: &field.ty,
                            type_ident: field.ident.as_ref().expect("field ident should exist"),
                            unpack_ident: format_ident!("a{i}"),
                        });

                        b_idents.push(NamedField {
                            ty: &field.ty,
                            type_ident: field.ident.as_ref().expect("field ident should exist"),
                            unpack_ident: format_ident!("b{i}"),
                        });
                    }

                    let a_unpacked: Vec<_> = a_idents
                        .iter()
                        .map(|a| {
                            let ident = a.type_ident;
                            let unpack = &a.unpack_ident;

                            quote! {
                                #ident: #unpack
                            }
                        })
                        .collect();

                    let b_unpacked = b_idents.iter().map(|b| {
                        let ident = b.type_ident;
                        let unpack = &b.unpack_ident;

                        quote! {
                            #ident: #unpack
                        }
                    });

                    let diff_statements = a_idents.iter().zip(b_idents.iter()).enumerate().map(
                        |(i, (a, b))| {
                            let i = i as u32;
                            let ty = a.ty;
                            let a = &a.unpack_ident;
                            let b = &b.unpack_ident;

                            quote! {
                                <#ty as #diff_path::Diff>::diff(#a, #b, path.with(#i), event_queue);
                            }
                        },
                    );

                    arms.push(quote! {
                        (#identifier::#variant_ident{#(#a_unpacked),*}, #identifier::#variant_ident{#(#b_unpacked),*}) => {
                            let path = path.with(#variant_index);

                            #(#diff_statements)*
                        }
                    });

                    let set_items = a_idents.iter().map(|a| {
                        let ty = &a.ty;
                        let a = &a.unpack_ident;

                        quote! { <#ty as ::core::clone::Clone>::clone(&#a) }
                    });

                    arms.push(quote! {
                        (#identifier::#variant_ident{#(#a_unpacked),*}, _) => {
                            event_queue.push_param(
                                #firewheel_path::event::ParamData::any((#(#set_items,)*)),
                                path.with(#variant_index),
                            );
                        }
                    })
                }
            }
        }

        let body = quote! {
            match (self, baseline) {
                #(#arms)*
            }
        };

        Ok(Self {
            body,
            bounds: types
                .iter()
                .map(|ty| {
                    let span = ty.span();
                    quote_spanned! {span=> #ty: #diff_path::Diff + ::core::clone::Clone + ::core::marker::Send + ::core::marker::Sync + 'static }
                })
                .collect(),
        })
    }
}
