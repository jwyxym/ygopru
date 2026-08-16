use darling::FromMeta;
use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote;
use syn::parse_macro_input;
use syn::Attribute;
use syn::DeriveInput;

#[derive(Default, FromMeta)]
struct MessageParameters {
    #[darling(default)]
    flag: Option<u8>,
    #[darling(default)]
    mod_name: Option<String>,
}

enum Direction {
    Standard(&'static str),
    Other(String),
}

pub fn ygopro_message(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match _ygopro_message(input) {
        Ok(stream) => stream.into(),
        Err(s) => {
            let error = syn::Error::new(Span::call_site(), s).to_compile_error();
            quote!(#error).into()
        }
    }
}

fn parse_parameters(attrs: &[Attribute]) -> darling::Result<(Direction, MessageParameters)> {
    let attr = attrs.iter().find(|attr| attr.path.is_ident("message"))
        .ok_or_else(|| darling::Error::custom("missing #[message(...)] attribute"))?;
    let list = match attr.parse_meta().map_err(darling::Error::custom)? {
        syn::Meta::List(list) => list,
        other => return Err(darling::Error::custom(format!("expected #[message(...)], got {other:?}"))),
    };
    let nested: Vec<syn::NestedMeta> = list.nested.into_iter().collect();

    let mut direction = None;
    let named: Vec<syn::NestedMeta> = nested.into_iter().filter_map(|item| match item {
        syn::NestedMeta::Meta(syn::Meta::Path(path)) => {
            let name = path.get_ident().map(|ident| ident.to_string()).unwrap_or_default();
            direction = Some(match name.as_str() {
                "ctos" => Direction::Standard("CTOS"),
                "stoc" => Direction::Standard("STOC"),
                "gm" => Direction::Standard("GM"),
                other => Direction::Other(other.to_string()),
            });
            None
        }
        other => Some(other),
    }).collect();

    let parameters = MessageParameters::from_list(&named)?;
    let direction = direction.ok_or_else(|| darling::Error::custom("Don't specify a direction."))?;
    Ok((direction, parameters))
}

pub fn _ygopro_message(input: DeriveInput) -> Result<proc_macro2::TokenStream, String> {
    let struct_ident = input.ident;
    let attributes = input.attrs;
    let (direction, parameters) = parse_parameters(&attributes).map_err(|err| format!("Cannot parse message paramter:\n{:?}", err))?;
    let mod_name = match parameters.mod_name {
        Some(name) => {
            let ident = Ident::from_string(&name).map_err(|err| format!("Illegal mod name identifier:\n{:?}", err))?;
            quote!(#ident)
        }
        None => if std::env::var("CARGO_PKG_NAME").as_deref() == Ok("ygopro-data") {
            quote!(crate)
        } else {
            quote!(::ygopro_data)
        },
    };

    let stream = match direction {
        Direction::Other(tag) => {
            let flag = match parameters.flag {
                Some(flag) => flag,
                None => return Err("Don't offer a flag".to_string()),
            };
            quote!{
                impl #mod_name::message::PureMessage for #struct_ident {}
                impl #mod_name::message::Message for #struct_ident {
                    fn message_type() -> #mod_name::message::all::MessageType {
                        #mod_name::message::all::MessageType::Other(#tag, #flag)
                    }
                }
            }
        }
        Direction::Standard(direction) => {
            let ident = Ident::from_string(direction).unwrap();
            let lower_ident = Ident::from_string(&direction.to_lowercase()).map_err(|err| format!("Illegal direction:\n{:?}", err))?;
            quote!{
                impl #mod_name::message::PureMessage for #struct_ident {}
                impl #mod_name::message::Message for #struct_ident {
                    fn message_type() -> #mod_name::message::all::MessageType {
                        #mod_name::message::all::MessageType::#ident(#mod_name::message::#lower_ident::MessageType::#struct_ident)
                    }
                }
            }
        }
    };
    Ok(stream)
}
