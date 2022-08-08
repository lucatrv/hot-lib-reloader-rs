use syn::{
    spanned::Spanned, token, ForeignItemFn, Ident, Item, ItemMacro, LitStr, Macro, Result,
    Visibility,
};

use super::code_gen::gen_hot_module_function_for;
use crate::util::read_unmangled_functions_from_file;

use super::{code_gen::generate_lib_loader_items, HotModuleAttribute};

pub(crate) struct HotModule {
    pub(crate) vis: Visibility,
    pub(crate) ident: Ident,
    pub(crate) items: Vec<Item>,
    pub(crate) attributes: Option<super::HotModuleAttribute>,
}

/// Parses something like
/// ```ignore
/// #[hot_module(name = "lib")]
/// mod foo {
///   /* ... */
///   hot_functions_from_file!("../lib/src/lib.rs");
///   /* ... */
///   #[hot_function]
///       pub fn do_stuff(arg: &str) -> u32 { /*generated*/ }
///   /* ... */
///   #[hot_functions]
///   extern "Rust" {
///       pub fn do_stuff(arg: &str) -> u32;
///   }
/// }
/// ```
impl syn::parse::Parse for HotModule {
    fn parse(stream: syn::parse::ParseStream) -> Result<Self> {
        let vis = stream
            .parse::<syn::Visibility>()
            .unwrap_or(Visibility::Inherited);

        stream.parse::<token::Mod>()?;

        let ident = stream.parse::<Ident>()?;

        let module_body_stream;
        syn::braced!(module_body_stream in stream);

        let mut items = Vec::new();

        while !module_body_stream.is_empty() {
            let item = module_body_stream.parse::<syn::Item>()?;

            match item {
                // parses the hot_functions_from_file!("path/to/file.rs") marker
                Item::Macro(ItemMacro {
                    mac: Macro { path, tokens, .. },
                    ..
                }) if path.is_ident("hot_functions_from_file") => {
                    let file_name: LitStr = syn::parse(tokens.into())?;
                    let functions = read_unmangled_functions_from_file(file_name)?;
                    for (f, span) in functions {
                        let f = gen_hot_module_function_for(f, span)?;
                        items.push(Item::Fn(f));
                    }
                }

                // parses and code gens
                // #[hot_function]
                // fn do_stuff(arg: &str) -> u32 {}
                syn::Item::Fn(func)
                    if func
                        .attrs
                        .iter()
                        .any(|attr| attr.path.is_ident("hot_function")) =>
                {
                    let span = func.span();
                    let f = ForeignItemFn {
                        attrs: Vec::new(),
                        vis: func.vis,
                        sig: func.sig,
                        semi_token: token::Semi::default(),
                    };
                    let f = gen_hot_module_function_for(f, span)?;
                    items.push(Item::Fn(f));
                }

                // parses and code gens
                // #[hot_functions]
                // extern "Rust" {
                //     pub fn do_stuff(arg: &str) -> u32;
                // }
                syn::Item::ForeignMod(foreign_mod)
                    if foreign_mod
                        .attrs
                        .iter()
                        .any(|attr| attr.path.is_ident("hot_functions")) =>
                {
                    for item in foreign_mod.items {
                        match item {
                            syn::ForeignItem::Fn(f) => {
                                let span = f.span();
                                let f = gen_hot_module_function_for(f, span)?;
                                items.push(Item::Fn(f));
                            }
                            _ => {
                                eprintln!(
                                    "[warn] hot_functions extern block includes unexpected items"
                                );
                            }
                        }
                    }
                }

                // otherwise just use the item as is
                item => items.push(item),
            };
        }

        Ok(Self {
            ident,
            vis,
            items,
            attributes: None,
        })
    }
}

impl quote::ToTokens for HotModule {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            vis,
            ident,
            items,
            attributes,
        } = self;

        let HotModuleAttribute { lib_name, lib_dir } = match attributes {
            None => panic!("Expected to have macro attributes"),
            Some(attributes) => attributes,
        };

        let lib_loader = generate_lib_loader_items(lib_dir, lib_name, tokens.span())
            .expect("error generating hot lib loader helpers");

        let module_def = quote::quote! {
            #vis mod #ident {
                #( #items )*

                #lib_loader
            }
        };

        proc_macro2::TokenStream::extend(tokens, module_def);
    }
}
