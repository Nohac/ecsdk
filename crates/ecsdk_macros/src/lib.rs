use heck::ToSnakeCase;
use proc_macro::TokenStream;
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
                #[derive(Component, Clone, serde::Serialize, serde::Deserialize)]
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
            pub fn insert_marker(self, entity: &mut bevy::ecs::system::EntityCommands<'_>) {
                match self {
                    #(#entity_commands_arms)*
                }
            }

            pub fn insert_marker_world(self, entity: &mut bevy::ecs::world::EntityWorldMut<'_>) {
                match self {
                    #(#entity_world_mut_arms)*
                }
            }

            pub fn replicate_markers(app: &mut bevy::app::App) {
                use bevy_replicon::prelude::AppRuleExt as _;

                #(#replicate_marker_calls)*
            }
        }

        #vis mod #module_ident {
            use bevy::ecs::prelude::*;
            use bevy::ecs::{lifecycle::HookContext, world::DeferredWorld};
            use ecsdk_core::WakeSignal;

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
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "ClientRequest only supports types without generics",
        ));
    }

    let ident = &input.ident;
    let response = find_request_response(input)?;

    Ok(quote! {
        impl ecsdk_replicon::ClientRequest for #ident {
            type Response = #response;
        }

        impl #ident {
            pub fn register(app: &mut bevy::app::App)
            where
                for<'a> <Self as bevy::prelude::Event>::Trigger<'a>: Default,
                for<'a> <#response as bevy::prelude::Event>::Trigger<'a>: Default,
            {
                <Self as ecsdk_replicon::ClientRequest>::register(app);
            }

            pub fn reply(
                commands: &mut bevy::ecs::prelude::Commands,
                client_id: bevy_replicon::prelude::ClientId,
                response: #response,
            ) {
                <Self as ecsdk_replicon::ClientRequest>::reply(commands, client_id, response);
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
