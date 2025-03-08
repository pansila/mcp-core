use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use std::collections::HashMap;
use syn::{
    parse::Parse, parse::ParseStream, parse_macro_input, punctuated::Punctuated, Expr, ExprLit,
    FnArg, ItemFn, Lit, Meta, Pat, PatType, Token, Type,
};

struct MacroArgs {
    name: Option<String>,
    description: Option<String>,
    param_descriptions: HashMap<String, String>,
}

impl Parse for MacroArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut description = None;
        let mut param_descriptions = HashMap::new();

        let meta_list: Punctuated<Meta, Token![,]> = Punctuated::parse_terminated(input)?;

        for meta in meta_list {
            match meta {
                Meta::NameValue(nv) => {
                    let ident = nv.path.get_ident().unwrap().to_string();
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Str(lit_str),
                        ..
                    }) = nv.value
                    {
                        match ident.as_str() {
                            "name" => name = Some(lit_str.value()),
                            "description" => description = Some(lit_str.value()),
                            _ => {}
                        }
                    }
                }
                Meta::List(list) if list.path.is_ident("params") => {
                    let nested: Punctuated<Meta, Token![,]> =
                        list.parse_args_with(Punctuated::parse_terminated)?;

                    for meta in nested {
                        if let Meta::NameValue(nv) = meta {
                            if let Expr::Lit(ExprLit {
                                lit: Lit::Str(lit_str),
                                ..
                            }) = nv.value
                            {
                                let param_name = nv.path.get_ident().unwrap().to_string();
                                param_descriptions.insert(param_name, lit_str.value());
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(MacroArgs {
            name,
            description,
            param_descriptions,
        })
    }
}

#[proc_macro_attribute]
pub fn tool(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as MacroArgs);
    let input_fn = parse_macro_input!(input as ItemFn);

    let fn_name = &input_fn.sig.ident;
    let fn_name_str = fn_name.to_string();
    let struct_name = format_ident!("{}", { fn_name_str.to_case(Case::Pascal) });
    let tool_name = args.name.unwrap_or(fn_name_str);
    let tool_description = args.description.unwrap_or_default();

    let mut param_defs = Vec::new();
    let mut param_names = Vec::new();
    let mut required_params = Vec::new();

    for arg in input_fn.sig.inputs.iter() {
        if let FnArg::Typed(PatType { pat, ty, .. }) = arg {
            if let Pat::Ident(param_ident) = &**pat {
                let param_name = &param_ident.ident;
                let param_name_str = param_name.to_string();
                let description = args
                    .param_descriptions
                    .get(&param_name_str)
                    .map(|s| s.as_str())
                    .unwrap_or("");

                param_names.push(param_name);

                // Check if the parameter type is Option<T>
                let is_optional = if let Type::Path(type_path) = &**ty {
                    type_path
                        .path
                        .segments
                        .last()
                        .map_or(false, |segment| segment.ident == "Option")
                } else {
                    false
                };

                if !is_optional {
                    required_params.push(param_name_str.clone());
                }

                param_defs.push(quote! {
                    #[schemars(description = #description)]
                    #param_name: #ty
                });
            }
        }
    }

    let params_struct_name = format_ident!("{}Parameters", struct_name);
    let expanded = quote! {
        #[derive(serde::Deserialize, schemars::JsonSchema)]
        struct #params_struct_name {
            #(#param_defs,)*
        }

        #input_fn

        #[derive(Default)]
        pub struct #struct_name;

        impl #struct_name {
            pub fn tool() -> mcp_core::types::Tool {
                let schema = schemars::schema_for!(#params_struct_name);
                let mut schema = serde_json::to_value(schema.schema).unwrap_or_default();
                if let serde_json::Value::Object(ref mut map) = schema {
                    map.insert("required".to_string(), serde_json::Value::Array(
                        vec![#(serde_json::Value::String(#required_params.to_string())),*]
                    ));
                }

                mcp_core::types::Tool {
                    name: #tool_name.to_string(),
                    description: Some(#tool_description.to_string()),
                    input_schema: schema,
                }
            }

            pub fn call() -> mcp_core::tools::ToolHandlerFn {
                move |req: mcp_core::types::CallToolRequest| {
                    Box::pin(async move {
                        let params = match req.arguments {
                            Some(args) => serde_json::to_value(args).unwrap_or_default(),
                            None => serde_json::Value::Null,
                        };

                        let params: #params_struct_name = match serde_json::from_value(params) {
                            Ok(p) => p,
                            Err(e) => return mcp_core::types::CallToolResponse {
                                content: vec![mcp_core::types::ToolResponseContent::Text {
                                    text: format!("Invalid parameters: {}", e)
                                }],
                                is_error: Some(true),
                                meta: None,
                            },
                        };

                        match #fn_name(#(params.#param_names,)*).await {
                            Ok(response) => {
                                let content = if let Ok(vec_content) = serde_json::from_value::<Vec<mcp_core::types::ToolResponseContent>>(
                                    serde_json::to_value(&response).unwrap_or_default()
                                ) {
                                    vec_content
                                } else if let Ok(single_content) = serde_json::from_value::<mcp_core::types::ToolResponseContent>(
                                    serde_json::to_value(&response).unwrap_or_default()
                                ) {
                                    vec![single_content]
                                } else {
                                    vec![mcp_core::types::ToolResponseContent::Text {
                                        text: format!("Invalid response type: {:?}", response)
                                    }]
                                };

                                mcp_core::types::CallToolResponse {
                                    content,
                                    is_error: None,
                                    meta: None,
                                }
                            },
                            Err(e) => mcp_core::types::CallToolResponse {
                                content: vec![mcp_core::types::ToolResponseContent::Text {
                                    text: format!("Tool execution error: {}", e)
                                }],
                                is_error: Some(true),
                                meta: None,
                            },
                        }
                    })
                }
            }
        }
    };

    TokenStream::from(expanded)
}
