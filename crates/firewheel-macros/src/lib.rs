extern crate proc_macro;

use bevy_macro_utils::fq_std::{FQOption, FQResult};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;

fn derive_param_inner(input: TokenStream) -> syn::Result<TokenStream2> {
    let input: syn::DeriveInput = syn::parse(input)?;
    let identifier = &input.ident;

    let syn::Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "`AudioParam` can only be derived on structs",
        ));
    };

    let syn::Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new(
            input.span(),
            "`AudioParam` can only be derived on structs with named fields",
        ));
    };

    let fields: Vec<_> = fields
        .named
        .iter()
        .map(|f| (f.ident.as_ref().unwrap(), &f.ty))
        .collect();

    let messages = fields.iter().enumerate().map(|(i, (identifier, _))| {
        let index = i as u16;
        quote! {
            self.#identifier.to_messages(&cmp.#identifier, &mut writer, path.with(#index));
        }
    });

    let patches = fields.iter().enumerate().map(|(i, (identifier, _))| {
        let index = i as u16;
        quote! {
            #FQOption::Some(#index) => self.#identifier.patch(data, &path[1..])
        }
    });

    let ticks = fields.iter().map(|(identifier, _)| {
        quote! {
            self.#identifier.tick(time);
        }
    });

    let (impl_generics, ty_generics, where_generics) = input.generics.split_for_impl();

    let mut where_generics = where_generics.cloned().unwrap_or_else(|| syn::WhereClause {
        where_token: Default::default(),
        predicates: Default::default(),
    });

    let param_path = quote! { ::firewheel_core::node };

    for (_, ty) in &fields {
        where_generics
            .predicates
            .push(syn::parse2(quote! { #ty: #param_path::AudioParam }).unwrap());
    }

    Ok(quote! {
        impl #impl_generics #param_path::AudioParam for #identifier #ty_generics #where_generics {
            fn to_messages(&self, cmp: &Self, mut writer: impl FnMut(#param_path::ParamEvent), path: #param_path::ParamPath) {
                #(#messages)*
            }

            fn patch(&mut self, data: &mut #param_path::ParamData, path: &[u16]) -> #FQResult<(), #param_path::PatchError> {
                match path.first() {
                    #(#patches,)*
                    _ => #FQResult::Err(#param_path::PatchError::InvalidPath),
                }
            }

            fn tick(&mut self, time: ::firewheel_core::clock::ClockSeconds) {
                #(#ticks)*
            }
        }
    })
}

#[proc_macro_derive(AudioParam)]
pub fn derive_audio_param(input: TokenStream) -> TokenStream {
    derive_param_inner(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
