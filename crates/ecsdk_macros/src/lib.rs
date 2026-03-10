use heck::ToSnakeCase;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, parse_macro_input};

#[proc_macro_derive(StateComponent)]
pub fn derive_state_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand_state_component(&input) {
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
                #[derive(Component, Clone)]
                #[component(on_insert = #hook_ident)]
                pub struct #variant_ident;

                fn #hook_ident(world: DeferredWorld, ctx: HookContext) {
                    sync_state(world, ctx.entity, super::#enum_ident::#variant_ident);
                }
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        #vis mod #module_ident {
            use bevy::ecs::prelude::*;
            use bevy::ecs::{lifecycle::HookContext, world::DeferredWorld};

            fn sync_state(mut world: DeferredWorld, entity: Entity, state: super::#enum_ident) {
                world.commands().entity(entity).insert(state);
            }

            #(#variants)*
        }
    })
}
