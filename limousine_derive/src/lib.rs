//! This crate contains macros to materialize hybrid index designs as part of
//! the limousine engine.
//!
//! This crate should not be imported directly, (it will not work) rather the macros
//! should be accessed through the [`limousine_engine`](https://crates.io/crates/limousine_engine)
//! crate.

#![deny(missing_docs)]
mod component;
mod layout;
mod util;

use layout::IndexLayout;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Ident;

/// Macro to materialize an hybrid index structure. To use, specify
/// a name for the generated structure and a layout. The layout consists of
/// a list of layer types, ordered from highest to lowest; for example:
///
/// ```ignore
/// // Note: this is just a syntax example, this macro needs to be called
/// // from the [`limousine_engine`](https://crates.io/crates/limousine_engine) crate.
///
/// create_hybrid_index! {
///     name: MyHybridIndex,
///     layout: {
///         pgm(4),
///         pgm(4),
///         btree(32),          // base layer
///     }
/// }
/// ```
/// Optionally, we can also specify the top component, which is allowed to grow vertically
/// indefinitely. By default, this is set to the ```BTreeMap``` structure from the Rust standard
/// library, however alternative implmentations based LSM and LSH trees are also provided.
///
/// TODO: example
///
/// The supported component types in the layout are:
///
/// 1. **btree(fanout: usize)**
/// 2. **disk_btree(fanout: usize)**
/// 3. **pgm(epsilon: usize)**
///
/// Note that not all layouts are valid; for instance trying to place a disk layer over an
/// in-memory layer will result in an error. These rules are enforced automatically by the macro.
/// The macro will generate a structure with the provided name, alongside an implementation of the
/// `Index` trait.
#[proc_macro]
pub fn create_hybrid_index(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    // Parse hybrid index description
    let layout = syn::parse_macro_input!(input as IndexLayout);
    let name = layout.name();

    let mod_name = proc_macro2::Ident::new(
        format!("__implmentation_{}", name.to_string().to_lowercase()).as_str(),
        proc_macro2::Span::call_site(),
    );

    let (alias_body, alias) = create_type_aliases(&layout);
    let (index_body, index_fields) = create_index_struct(&layout, &alias);

    let mut implementation = proc_macro2::TokenStream::new();
    implementation.extend(quote! {
        pub mod #mod_name {
            use ::limousine_engine::private::*;

            #(#alias_body)*

            #index_body
        }

        use #mod_name::#name;
    });

    implementation.into()
}

fn create_type_aliases(layout: &IndexLayout) -> (Vec<TokenStream>, Vec<Ident>) {
    let mut type_alias = Vec::new();
    let mut type_alias_body = Vec::new();

    // Add body as the first component
    type_alias.push(Ident::new("Component0", Span::call_site()));

    let alias = type_alias[0].clone();
    let body = layout.base.to_tokens();
    type_alias_body.push(quote::quote! {
        type #alias<K, V> = #body;
    });

    // Add internal components
    for (mut index, component) in layout.internal.iter().rev().enumerate() {
        index += 1;

        type_alias.push(Ident::new(
            format!("Component{}", index).as_str(),
            Span::call_site(),
        ));

        let previous_alias = type_alias[index - 1].clone();
        let body = component.to_tokens(quote! { #previous_alias<K, V> });

        let alias = type_alias[index].clone();
        type_alias_body.push(quote! {
            type #alias<K, V> = #body;
        });
    }

    // Add top component
    let index = layout.internal.len() + 1;
    type_alias.push(Ident::new(
        format!("Component{}", index).as_str(),
        Span::call_site(),
    ));

    let previous_alias = type_alias[index - 1].clone();
    let body = layout.top.to_tokens(quote! { #previous_alias<K, V> });

    let alias = type_alias[index].clone();
    type_alias_body.push(quote! {
        type #alias<K, V> = #body;
    });

    (type_alias_body, type_alias)
}

fn create_index_struct(layout: &IndexLayout, alias: &Vec<Ident>) -> (TokenStream, Vec<Ident>) {
    let name = layout.name();

    // Create fields
    let mut fields = Vec::new();
    for component in alias.iter() {
        fields.push(Ident::new(
            component.to_string().to_lowercase().as_str(),
            Span::call_site(),
        ));
    }

    // Create field definitions
    let mut field_bodies = Vec::new();
    for (index, component) in alias.iter().enumerate() {
        let field = fields[index].clone();
        field_bodies.push(quote! {
            #field: #component<K, V>,
        });
    }

    let body = quote! {
        pub struct #name<K: Key, V: Value> {
            #(#field_bodies)*
        }
    };

    (body, fields)
}
