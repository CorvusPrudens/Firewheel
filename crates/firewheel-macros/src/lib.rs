extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};

mod diff;
mod firewheel_manifest;
mod patch;

#[proc_macro_derive(Diff, attributes(diff))]
pub fn derive_diff(input: TokenStream) -> TokenStream {
    diff::derive_diff(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_derive(Patch, attributes(diff))]
pub fn derive_patch(input: TokenStream) -> TokenStream {
    patch::derive_patch(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
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

fn struct_fields(data: &syn::DataStruct) -> Vec<(TokenStream2, &syn::Type)> {
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

    fields
}
