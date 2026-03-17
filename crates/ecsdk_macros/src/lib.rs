//! Proc macros for `ecsdk`.
//!
//! These derives assume consumers depend on the public `ecsdk` facade crate.

use heck::ToSnakeCase;
use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, LitStr, Path, parse_macro_input};

#[proc_macro_derive(StateComponent)]
pub fn derive_state_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand_state_component(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(ClientRequest, attributes(request))]
pub fn derive_client_request(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand_client_request(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_state_component(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ecsdk = ecsdk_path();

    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "StateComponent only supports enums without generics",
        ));
    }

    let Data::Enum(data_enum) = &input.data else {
        return Err(syn::Error::new_spanned(
            input,
            "StateComponent can only be derived for enums",
        ));
    };

    let enum_ident = &input.ident;
    let module_ident = format_ident!("{}", enum_ident.to_string().to_snake_case());
    let vis = &input.vis;

    let variants = data_enum
        .variants
        .iter()
        .map(|variant| {
            if !matches!(variant.fields, Fields::Unit) {
                return Err(syn::Error::new_spanned(
                    &variant.fields,
                    "StateComponent only supports fieldless enum variants",
                ));
            }

            let variant_ident = &variant.ident;
            let hook_ident = format_ident!("on_insert_{}", variant_ident.to_string().to_snake_case());

            Ok(quote! {
                #[derive(Component, Clone, #ecsdk::serde::Serialize, #ecsdk::serde::Deserialize)]
                #[component(immutable, on_insert = #hook_ident)]
                pub struct #variant_ident;

                fn #hook_ident(world: DeferredWorld, ctx: HookContext) {
                    sync_state(world, ctx.entity, super::#enum_ident::#variant_ident);
                }
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    let entity_commands_arms = data_enum
        .variants
        .iter()
        .map(|variant| {
            let variant_ident = &variant.ident;
            quote! {
                Self::#variant_ident => {
                    entity.insert(#module_ident::#variant_ident);
                }
            }
        })
        .collect::<Vec<_>>();

    let entity_world_mut_arms = data_enum
        .variants
        .iter()
        .map(|variant| {
            let variant_ident = &variant.ident;
            quote! {
                Self::#variant_ident => {
                    entity.insert(#module_ident::#variant_ident);
                }
            }
        })
        .collect::<Vec<_>>();

    let replicate_marker_calls = data_enum
        .variants
        .iter()
        .map(|variant| {
            let variant_ident = &variant.ident;
            quote! {
                app.replicate::<#module_ident::#variant_ident>();
            }
        })
        .collect::<Vec<_>>();

    Ok(quote! {
        impl #enum_ident {
            pub fn insert_marker(self, entity: &mut #ecsdk::bevy::ecs::system::EntityCommands<'_>) {
                match self {
                    #(#entity_commands_arms)*
                }
            }

            pub fn insert_marker_world(self, entity: &mut #ecsdk::bevy::ecs::world::EntityWorldMut<'_>) {
                match self {
                    #(#entity_world_mut_arms)*
                }
            }

            pub fn replicate_markers(app: &mut #ecsdk::bevy::app::App) {
                use #ecsdk::bevy_replicon::prelude::AppRuleExt as _;

                #(#replicate_marker_calls)*
            }
        }

        #vis mod #module_ident {
            use #ecsdk::bevy::ecs::prelude::*;
            use #ecsdk::bevy::ecs::{lifecycle::HookContext, world::DeferredWorld};
            use #ecsdk::core::WakeSignal;

            fn sync_state(mut world: DeferredWorld, entity: Entity, state: super::#enum_ident) {
                world.commands().entity(entity).insert(state);
                if let Some(wake) = world.get_resource::<WakeSignal>() {
                    wake.0.notify_one();
                }
            }

            #(#variants)*
        }
    })
}

fn expand_client_request(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
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
        impl #ecsdk::replicon::ClientRequest for #ident {
            type Response = #response;
        }

        impl #ident {
            pub fn register(app: &mut #ecsdk::bevy::app::App)
            where
                for<'a> <Self as #ecsdk::bevy::prelude::Event>::Trigger<'a>: Default,
                for<'a> <#response as #ecsdk::bevy::prelude::Event>::Trigger<'a>: Default,
            {
                <Self as #ecsdk::replicon::ClientRequest>::register(app);
            }

            pub fn reply(
                commands: &mut #ecsdk::bevy::ecs::prelude::Commands,
                client_id: #ecsdk::bevy_replicon::prelude::ClientId,
                response: #response,
            ) {
                <Self as #ecsdk::replicon::ClientRequest>::reply(commands, client_id, response);
            }
        }
    })
}

fn ecsdk_path() -> Path {
    match crate_name("ecsdk") {
        Ok(FoundCrate::Itself) => syn::parse_quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            syn::parse_quote!(#ident)
        }
        Err(_) => syn::parse_quote!(ecsdk),
    }
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
