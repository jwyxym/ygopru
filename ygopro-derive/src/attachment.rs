use darling::FromMeta;
use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;
use syn::Attribute;
use syn::DeriveInput;

#[derive(Default, FromMeta)]
struct AttachmentArgs {
    #[darling(default)]
    no_default: bool,
}

#[derive(Default, FromMeta)]
struct FieldArgs {
    #[darling(default)]
    default: Option<syn::LitStr>,
}

pub fn attachment(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();
    let args = match parse_struct_args(&input.attrs) {
        Ok(args) => args,
        Err(err) => return err.write_errors().into(),
    };
    let fields = match &input.data {
        syn::Data::Struct(syn::DataStruct { fields: syn::Fields::Named(named), .. }) => named.named.iter().collect::<Vec<_>>(),
        _ => return darling::Error::custom("Attachment derive requires a named-field struct").write_errors().into(),
    };
    let mut field_defaults = Vec::new();
    for field in fields {
        let ident = field.ident.clone().unwrap();
        let default = match field_default_expr(field) {
            Ok(Some(tokens)) => tokens,
            Ok(None) => quote!(Default::default()),
            Err(err) => return err.write_errors().into(),
        };
        field_defaults.push(quote! { #ident: #default });
    }
    let default_attachment = if args.no_default {
        quote!()
    } else {
        quote! {
            impl #name {
                fn default_attachment() -> Self {
                    Self {
                        #(#field_defaults),*
                    }
                }
            }
        }
    };
    let extraction = if args.no_default {
        quote! {
            let attachment = map.get_mut::<#name>()?;
        }
    } else {
        quote! {
            let attachment = map.entry::<#name>().or_insert_with(#name::default_attachment);
        }
    };
    quote! {
        #default_attachment

        impl<Req, State, Res> ::ygopro_handler::FromRequest<Req, State, Res> for &mut #name
        where
            Req: Send,
            State: Send,
            Res: Send,
            State: ::ygopro_handler::extract::ContainsMapMut,
        {
            fn from_request(bundle: &mut ::ygopro_handler::Bundle<Req, State, Res>) -> Option<Self> {
                let map = ::ygopro_handler::extract::ContainsMapMut::get_map(&mut bundle.state);
                #extraction
                Some(unsafe { &mut *(attachment as *mut #name) })
            }
        }
    }
    .into()
}

fn parse_struct_args(attrs: &[Attribute]) -> darling::Result<AttachmentArgs> {
    match attrs.iter().find(|attr| attr.path.is_ident("attachment")) {
        Some(attr) => {
            let meta: syn::Meta = attr.parse_meta().map_err(darling::Error::custom)?;
            match meta {
                syn::Meta::List(list) => {
                    let nested: Vec<syn::NestedMeta> = list.nested.into_iter().collect();
                    AttachmentArgs::from_list(&nested)
                },
                other => Err(darling::Error::custom(format!("expected #[attachment(...)], got {other:?}"))),
            }
        }
        None => Ok(AttachmentArgs::default()),
    }
}

fn field_default_expr(field: &syn::Field) -> darling::Result<Option<proc_macro2::TokenStream>> {
    match field.attrs.iter().find(|attr| attr.path.is_ident("attachment")) {
        Some(attr) => {
            let meta: syn::Meta = attr.parse_meta().map_err(darling::Error::custom)?;
            match meta {
                syn::Meta::List(list) => {
                    let nested: Vec<syn::NestedMeta> = list.nested.into_iter().collect();
                    let args = FieldArgs::from_list(&nested)?;
                    match &args.default {
                        Some(lit) => match syn::parse_str::<proc_macro2::TokenStream>(&lit.value()) {
                            Ok(tokens) => Ok(Some(tokens)),
                            Err(err) => Err(darling::Error::custom(format!("default must be a valid expression: {err}"))),
                        },
                        None => Ok(None),
                    }
                },
                _ => Ok(None),
            }
        }
        None => Ok(None),
    }
}
