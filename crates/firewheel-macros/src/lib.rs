extern crate proc_macro;

use bevy_macro_utils::fq_std::{FQOption, FQResult};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use syn::spanned::Spanned;

mod firewheel_manifest;

#[proc_macro_derive(Diff)]
pub fn derive_diff(input: TokenStream) -> TokenStream {
    derive_diff_inner(input, quote! { ::firewheel_core })
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn derive_diff_inner(
    input: TokenStream,
    firewheel_path: TokenStream2,
) -> syn::Result<TokenStream2> {
    let input: syn::DeriveInput = syn::parse(input)?;
    let identifier = &input.ident;

    let syn::Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "`Diff` can only be derived on structs",
        ));
    };

    // NOTE: a trivial optimization would be to automatically
    // flatten structs with only a single field so their
    // paths can be one index shorter.
    let fields: Vec<_> = match &data.fields {
        syn::Fields::Named(fields) => fields
            .named
            .iter()
            .map(|f| (f.ident.as_ref().unwrap().to_token_stream(), &f.ty))
            .collect(),
        syn::Fields::Unnamed(fields) => fields
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let accessor: syn::Index = i.into();
                (accessor.to_token_stream(), &f.ty)
            })
            .collect(),
        syn::Fields::Unit => Vec::new(),
    };

    let messages = fields.iter().enumerate().map(|(i, (identifier, _))| {
        let index = i as u32;
        quote! {
            self.#identifier.diff(&baseline.#identifier, path.with(#index), event_queue);
        }
    });

    let patches = fields.iter().enumerate().map(|(i, (identifier, _))| {
        let index = i as u32;
        quote! {
            #FQOption::Some(#index) => self.#identifier.patch(data, &path[1..])
        }
    });

    let (impl_generics, ty_generics, where_generics) = input.generics.split_for_impl();

    let mut where_generics = where_generics.cloned().unwrap_or_else(|| syn::WhereClause {
        where_token: Default::default(),
        predicates: Default::default(),
    });

    let firewheel_path =
        firewheel_manifest::FirewheelManifest::default().get_path("firewheel_core");
    let param_path = quote! { #firewheel_path::param };

    for (_, ty) in &fields {
        where_generics
            .predicates
            .push(syn::parse2(quote! { #ty: #param_path::Diff }).unwrap());
    }

    Ok(quote! {
        impl #impl_generics #param_path::Diff for #identifier #ty_generics #where_generics {
            fn diff<__E: #param_path::EventQueue>(&self, baseline: &Self, path: #param_path::PathBuilder, event_queue: &mut __E) {
                #(#messages)*
            }

            fn patch(&mut self, data: &#firewheel_path::event::ParamData, path: &[u32]) -> #FQResult<(), #param_path::PatchError> {
                match path.first() {
                    #(#patches,)*
                    _ => #FQResult::Err(#param_path::PatchError::InvalidPath),
                }
            }
        }
    })
}
