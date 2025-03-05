use bevy_macro_utils::fq_std::FQResult;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::{get_paths, struct_fields};

pub fn derive_patch(input: TokenStream) -> syn::Result<TokenStream2> {
    let input: syn::DeriveInput = syn::parse(input)?;
    let identifier = &input.ident;

    let fields = match &input.data {
        syn::Data::Struct(data) => struct_fields(data),
        _ => todo!(),
    };

    let patches = fields.iter().enumerate().map(|(i, (identifier, _))| {
        let index = i as u32;
        quote! {
            [#index, tail @ .. ] => self.#identifier.patch(data, tail)
        }
    });

    let (firewheel_path, diff_path) = get_paths();

    let (impl_generics, ty_generics, where_generics) = input.generics.split_for_impl();

    let mut where_generics = where_generics.cloned().unwrap_or_else(|| syn::WhereClause {
        where_token: Default::default(),
        predicates: Default::default(),
    });

    for (_, ty) in &fields {
        where_generics
            .predicates
            .push(syn::parse2(quote! { #ty: #diff_path::Patch }).unwrap());
    }

    Ok(quote! {
        impl #impl_generics #diff_path::Patch for #identifier #ty_generics #where_generics {
            fn patch(&mut self, data: &#firewheel_path::event::ParamData, path: &[u32]) -> #FQResult<(), #diff_path::PatchError> {
                match path {
                    #(#patches,)*
                    _ => #FQResult::Err(#diff_path::PatchError::InvalidPath),
                }
            }
        }
    })
}
