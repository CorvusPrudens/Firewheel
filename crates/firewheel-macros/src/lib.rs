extern crate proc_macro;

use bevy_macro_utils::fq_std::{FQOption, FQResult};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use syn::spanned::Spanned;

mod firewheel_manifest;

#[proc_macro_derive(Diff, attributes(diff))]
pub fn derive_diff(input: TokenStream) -> TokenStream {
    derive_diff_inner(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn derive_diff_inner(input: TokenStream) -> syn::Result<TokenStream2> {
    let input: syn::DeriveInput = syn::parse(input)?;
    let identifier = &input.ident;

    let fields = get_fields(&input)?;

    let messages = fields.iter().enumerate().map(|(i, (identifier, _))| {
        let index = i as u32;
        quote! {
            self.#identifier.diff(&baseline.#identifier, path.with(#index), event_queue);
        }
    });

    let (_, diff_path) = get_paths();

    let (impl_generics, ty_generics, where_generics) = input.generics.split_for_impl();

    let mut where_generics = where_generics.cloned().unwrap_or_else(|| syn::WhereClause {
        where_token: Default::default(),
        predicates: Default::default(),
    });

    for (_, ty) in &fields {
        where_generics
            .predicates
            .push(syn::parse2(quote! { #ty: #diff_path::Diff }).unwrap());
    }

    Ok(quote! {
        impl #impl_generics #diff_path::Diff for #identifier #ty_generics #where_generics {
            fn diff<__E: #diff_path::EventQueue>(&self, baseline: &Self, path: #diff_path::PathBuilder, event_queue: &mut __E) {
                #(#messages)*
            }
        }
    })
}

#[proc_macro_derive(Patch, attributes(diff))]
pub fn derive_patch(input: TokenStream) -> TokenStream {
    derive_patch_inner(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn derive_patch_inner(input: TokenStream) -> syn::Result<TokenStream2> {
    let input: syn::DeriveInput = syn::parse(input)?;
    let identifier = &input.ident;

    let fields = get_fields(&input)?;

    let patches = fields.iter().enumerate().map(|(i, (identifier, _))| {
        let index = i as u32;
        quote! {
            #FQOption::Some(#index) => self.#identifier.patch(data, &path[1..])
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
                match path.first() {
                    #(#patches,)*
                    _ => #FQResult::Err(#diff_path::PatchError::InvalidPath),
                }
            }
        }
    })
}

fn get_paths() -> (syn::Path, TokenStream2) {
    let firewheel_path =
        firewheel_manifest::FirewheelManifest::default().get_path("firewheel_core");
    let diff_path = quote! { #firewheel_path::diff };

    (firewheel_path, diff_path)
}

fn should_skip(attrs: &[syn::Attribute]) -> bool {
    let mut skip = false;
    for attr in attrs {
        if attr.path().is_ident("diff") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                    skip = true;
                }

                Ok(())
            })
            .expect("infallible operation");
        }
    }

    skip
}

fn get_fields(input: &syn::DeriveInput) -> syn::Result<Vec<(TokenStream2, &syn::Type)>> {
    let syn::Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "`Diff` and `Patch` can only be derived on structs",
        ));
    };

    // NOTE: a trivial optimization would be to automatically
    // flatten structs with only a single field so their
    // paths can be one index shorter.
    let fields: Vec<_> = match &data.fields {
        syn::Fields::Named(fields) => fields
            .named
            .iter()
            .filter(|f| !should_skip(&f.attrs))
            .map(|f| (f.ident.as_ref().unwrap().to_token_stream(), &f.ty))
            .collect(),
        syn::Fields::Unnamed(fields) => fields
            .unnamed
            .iter()
            .filter(|f| !should_skip(&f.attrs))
            .enumerate()
            .map(|(i, f)| {
                let accessor: syn::Index = i.into();
                (accessor.to_token_stream(), &f.ty)
            })
            .collect(),
        syn::Fields::Unit => Vec::new(),
    };

    Ok(fields)
}
