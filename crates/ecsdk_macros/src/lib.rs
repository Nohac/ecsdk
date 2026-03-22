//! Proc macros for `ecsdk`.
//!
//! These derives assume consumers depend on the public `ecsdk` facade crate.

mod client_request;
mod state_component;

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use syn::{DeriveInput, Path, parse_macro_input};

#[proc_macro_derive(StateComponent)]
pub fn derive_state_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match state_component::expand_state_component(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(ClientRequest, attributes(request))]
pub fn derive_client_request(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match client_request::expand_client_request(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

pub(crate) fn ecsdk_path() -> Path {
    match crate_name("ecsdk") {
        Ok(FoundCrate::Itself) => syn::parse_quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            syn::parse_quote!(#ident)
        }
        Err(_) => syn::parse_quote!(ecsdk),
    }
}
