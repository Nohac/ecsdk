use quote::quote;
use syn::{DeriveInput, LitStr, Path};

use crate::ecsdk_path;

pub(crate) fn expand_client_request(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ecsdk = ecsdk_path();

    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "ClientRequest only supports types without generics",
        ));
    }

    let ident = &input.ident;
    let response = find_request_response(input)?;

    Ok(quote! {
        impl #ecsdk::network::ClientRequest for #ident {
            type Response = #response;
        }

        impl #ident {
            pub fn register(app: &mut #ecsdk::bevy::app::App)
            where
                for<'a> <Self as #ecsdk::bevy::prelude::Event>::Trigger<'a>: Default,
                for<'a> <#response as #ecsdk::bevy::prelude::Event>::Trigger<'a>: Default,
            {
                <Self as #ecsdk::network::ClientRequest>::register(app);
            }

            pub fn reply(
                commands: &mut #ecsdk::bevy::ecs::prelude::Commands,
                client_id: #ecsdk::bevy_replicon::prelude::ClientId,
                response: #response,
            ) {
                <Self as #ecsdk::network::ClientRequest>::reply(commands, client_id, response);
            }
        }
    })
}

fn find_request_response(input: &DeriveInput) -> syn::Result<Path> {
    for attr in &input.attrs {
        if !attr.path().is_ident("request") {
            continue;
        }

        let mut response = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("response") {
                let value = meta.value()?;
                let lit: LitStr = value.parse()?;
                response = Some(lit.parse::<Path>()?);
                return Ok(());
            }
            Err(meta.error("unsupported request attribute"))
        })?;

        if let Some(response) = response {
            return Ok(response);
        }

        return Err(syn::Error::new_spanned(
            attr,
            "missing `response = \"Type\"` in request attribute",
        ));
    }

    Err(syn::Error::new_spanned(
        input,
        "ClientRequest requires `#[request(response = \"Type\")]`",
    ))
}
