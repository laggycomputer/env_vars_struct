//! A simple macro to define nested structs which are populated from a list of expected variables passed at compile-tine.
//!
//! Names are converted to `snake_case` and periods are treated as separators indicating a nested struct.
//! ```
//! use env_vars_struct::env_vars_struct;
//!
//! env_vars_struct!(
//!     "DATABASE.HOST",
//!     "DATABASE.PORT",
//!     "API.KEY",
//!     "API.SECRET",
//!     "CACHE.REDIS.URL",
//!     "HAT",
//! );
//!
//! // safety: no \0, =, or NUL here and nobody should do this in practice
//! unsafe {
//!     std::env::set_var("DATABASE_HOST", "host");
//!     std::env::set_var("DATABASE_PORT", "5432");
//!     std::env::set_var("API.KEY", "magic key");
//!     std::env::set_var("API.SECRET", "magic secret");
//!     std::env::set_var("CACHE.REDIS.URL", "redis://someplace");
//!     std::env::set_var("HAT", "fedora");
//! }
//!
//! let vars = Vars::new();
//! println!("db: {}:{}", vars.database.host, vars.database.port);
//! println!("redis: {}", vars.cache.redis.url);
//! println!("api: key {}, secret {}", vars.api.key, vars.api.secret);
//! println!("hat: {}", vars.hat);
//! ```

#![deny(clippy::missing_safety_doc)]
#![deny(unsafe_op_in_unsafe_fn)]

use proc_macro::TokenStream;
use quote::quote;
use std::collections::HashMap;
use syn::{LitStr, parse_macro_input};

struct EnvVarsInput {
    vars: Vec<String>,
}

const ROOT_STRUCT_NAME: &str = "Vars";

#[proc_macro]
pub fn env_vars_struct(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as EnvVarsInput);

    let mut root = Node::default();

    for var_name in &input.vars {
        let parts = var_name.split('.').collect::<Vec<_>>();
        insert_path(&mut root, &parts, var_name);
    }

    let structs = generate_structs(&root, ROOT_STRUCT_NAME);
    let root_struct = generate_root_struct(&root);

    let expanded = quote! {
        #structs
        #root_struct
    };

    expanded.into()
}

#[derive(Default)]
struct Node {
    children: HashMap<String, Node>,
    leaf_var: Option<String>,
}

fn insert_path(node: &mut Node, parts: &[&str], full_var: &str) {
    if parts.is_empty() {
        return;
    }

    if parts.len() == 1 {
        node.children
            .entry(parts[0].to_string())
            .or_default()
            .leaf_var = Some(full_var.to_string());
    } else {
        let child = node.children.entry(parts[0].to_string()).or_default();
        insert_path(child, &parts[1..], full_var);
    }
}

fn generate_structs(node: &Node, struct_name: &str) -> proc_macro2::TokenStream {
    let mut output = proc_macro2::TokenStream::new();

    for (field_name, child_node) in &node.children {
        if child_node.leaf_var.is_none() || !child_node.children.is_empty() {
            // intermediate node
            let child_struct_name = format!("{}{}", struct_name, to_pascal_case(field_name));
            let child_struct_ident =
                syn::Ident::new(&child_struct_name, proc_macro2::Span::call_site());

            let child_structs = generate_structs(child_node, &child_struct_name);
            output.extend(child_structs);

            let fields = generate_struct_fields(child_node, &child_struct_name);
            let field_inits = generate_field_inits(child_node, &child_struct_name);

            output.extend(quote! {
                #[derive(Debug, Clone)]
                pub struct #child_struct_ident {
                    #fields
                }

                impl #child_struct_ident {
                    fn new() -> Self {
                        Self {
                            #field_inits
                        }
                    }
                }
            });
        }
    }

    output
}

fn generate_root_struct(node: &Node) -> proc_macro2::TokenStream {
    let struct_name = syn::Ident::new(ROOT_STRUCT_NAME, proc_macro2::Span::call_site());
    let fields = generate_struct_fields(node, ROOT_STRUCT_NAME);
    let field_inits = generate_field_inits(node, ROOT_STRUCT_NAME);

    quote! {
        #[derive(Debug, Clone)]
        pub struct #struct_name {
            #fields
        }

        impl #struct_name {
            pub fn new() -> Self {
                Self {
                    #field_inits
                }
            }
        }

        impl Default for #struct_name {
            fn default() -> Self {
                Self::new()
            }
        }
    }
}

fn generate_struct_fields(node: &Node, parent_struct: &str) -> proc_macro2::TokenStream {
    let mut fields = proc_macro2::TokenStream::new();

    for (field_name, child_node) in &node.children {
        let field_ident =
            syn::Ident::new(&to_snake_case(field_name), proc_macro2::Span::call_site());

        if child_node.leaf_var.is_some() && child_node.children.is_empty() {
            // leaf
            fields.extend(quote! {
                pub #field_ident: String,
            });
        } else {
            // not a leaf
            let child_struct_name = format!("{}{}", parent_struct, to_pascal_case(field_name));
            let child_struct_ident =
                syn::Ident::new(&child_struct_name, proc_macro2::Span::call_site());

            fields.extend(quote! {
                pub #field_ident: #child_struct_ident,
            });
        }
    }

    fields
}

fn generate_field_inits(node: &Node, parent_struct: &str) -> proc_macro2::TokenStream {
    let mut inits = proc_macro2::TokenStream::new();

    for (field_name, child_node) in &node.children {
        let field_ident =
            syn::Ident::new(&to_snake_case(field_name), proc_macro2::Span::call_site());

        if let Some(var_name) = &child_node.leaf_var
            && child_node.children.is_empty()
        {
            // leaf
            inits.extend(quote! {
                #field_ident: std::env::var(#var_name)
                    .unwrap_or_else(|_| panic!("Environment variable {} not found", #var_name)),
            });
        } else {
            // has children
            let child_struct_name = format!("{}{}", parent_struct, to_pascal_case(field_name));
            let child_struct_ident =
                syn::Ident::new(&child_struct_name, proc_macro2::Span::call_site());

            inits.extend(quote! {
                #field_ident: #child_struct_ident::new(),
            });
        }
    }

    inits
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(|c| c.to_lowercase()))
                    .collect(),
            }
        })
        .collect()
}

fn to_snake_case(s: &str) -> String {
    s.to_lowercase().replace('-', "_")
}

impl syn::parse::Parse for EnvVarsInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut vars = Vec::new();

        while !input.is_empty() {
            let var: LitStr = input.parse()?;
            vars.push(var.value());

            if input.is_empty() {
                break;
            }

            let _: syn::Token![,] = input.parse()?;
        }

        Ok(EnvVarsInput { vars })
    }
}
