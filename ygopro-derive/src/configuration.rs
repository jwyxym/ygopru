use darling::FromMeta;
use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;
use syn::Attribute;
use syn::DeriveInput;

#[derive(FromMeta)]
struct ConfigurationArgs {
    #[darling(default)]
    register_to: Option<syn::LitStr>,
}

#[derive(Default, FromMeta)]
struct FieldConfigArgs {
    #[darling(default)]
    ignore: bool,
    #[darling(default)]
    default: Option<syn::LitStr>,
}

struct FieldConfig {
    ident: syn::Ident,
    default: proc_macro2::TokenStream,
    overridable: bool,
}

pub fn configuration(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();
    let args = match parse_args(&input.attrs) {
        Ok(args) => args,
        Err(err) => return err.write_errors().into(),
    };
    let register_to = match &args.register_to {
        Some(lit) => syn::parse_str::<syn::Path>(&lit.value()),
        None => syn::parse_str::<syn::Path>("crate::plugin::CONFIGURATIONS"),
    };
    let register_to = match register_to {
        Ok(path) => path,
        Err(err) => return darling::Error::custom(format!("register_to must be a path: {err}")).write_errors().into(),
    };
    let fields = match &input.data {
        syn::Data::Struct(syn::DataStruct { fields: syn::Fields::Named(named), .. }) => named.named.iter().collect::<Vec<_>>(),
        _ => return darling::Error::custom("Configuration derive requires a named-field struct").write_errors().into(),
    };
    let mut field_configs = Vec::new();
    for field in fields {
        let ident = field.ident.clone().unwrap();
        let field_args = match parse_field_args(field) {
            Ok(args) => args,
            Err(err) => return err.write_errors().into(),
        };
        let default = match &field_args.default {
            Some(lit) => match syn::parse_str::<proc_macro2::TokenStream>(&lit.value()) {
                Ok(tokens) => tokens,
                Err(err) => return darling::Error::custom(format!("default must be a valid expression: {err}")).write_errors().into(),
            },
            None => quote!(Default::default()),
        };
        field_configs.push(FieldConfig { ident, default, overridable: !field_args.ignore });
    }
    let struct_literal = field_configs.iter().map(|config| {
        let ident = &config.ident;
        let default = &config.default;
        quote! { #ident: #default }
    });
    let field_overrides: Vec<proc_macro2::TokenStream> = field_configs.iter().filter(|config| config.overridable).map(|config| {
        let ident = &config.ident;
        let name = syn::LitStr::new(&ident.to_string(), ident.span());
        quote! {
            configuration.#ident = config_manager
                .get(#name)
                .and_then(|value| value.parse().ok())
                .unwrap_or(configuration.#ident);
        }
    }).collect();
    let apply_overrides = if field_overrides.is_empty() {
        quote!()
    } else {
        quote! {
            if let Some(config_manager) = crate::managers::config_manager::load().as_ref() {
                #(#field_overrides)*
            }
        }
    };
    quote! {
        impl #name {
            fn default_configuration(configurations: &mut ::anymap3::Map<dyn ::anymap3::CloneAny + Send>) -> Result<(), Box<dyn std::error::Error>> {
                let mut configuration = #name {
                    #(#struct_literal),*
                };
                #apply_overrides
                configurations.insert(configuration);
                Ok(())
            }
        }

        #[linkme::distributed_slice(#register_to)]
        static CONFIGURATION: (&'static str, fn(&mut ::anymap3::Map<dyn ::anymap3::CloneAny + Send>) -> Result<(), Box<dyn std::error::Error>>) = (module_path!(), #name::default_configuration);

        impl<Req, State, Res> ::ygopro_handler::FromRequest<Req, State, Res> for #name
        where
            Req: Send,
            State: Send,
            Res: Send,
            State: ::ygopro_handler::extract::ContainsMap,
        {
            fn from_request(bundle: &mut ::ygopro_handler::Bundle<Req, State, Res>) -> Option<Self> {
                ::ygopro_handler::extract::ContainsMap::get_map(&bundle.state).get::<#name>().cloned()
            }
        }
    }
    .into()
}

fn parse_args(attrs: &[Attribute]) -> darling::Result<ConfigurationArgs> {
    match attrs.iter().find(|attr| attr.path.is_ident("config")) {
        Some(attr) => {
            let meta: syn::Meta = attr.parse_meta().map_err(darling::Error::custom)?;
            match meta {
                syn::Meta::List(list) => {
                    let nested: Vec<syn::NestedMeta> = list.nested.into_iter().collect();
                    ConfigurationArgs::from_list(&nested)
                },
                other => Err(darling::Error::custom(format!("expected #[config(...)], got {other:?}"))),
            }
        }
        None => Ok(ConfigurationArgs { register_to: None }),
    }
}

fn parse_field_args(field: &syn::Field) -> darling::Result<FieldConfigArgs> {
    match field.attrs.iter().find(|attr| attr.path.is_ident("config")) {
        Some(attr) => {
            let meta: syn::Meta = attr.parse_meta().map_err(darling::Error::custom)?;
            match meta {
                syn::Meta::List(list) => {
                    let nested: Vec<syn::NestedMeta> = list.nested.into_iter().collect();
                    FieldConfigArgs::from_list(&nested)
                },
                other => Err(darling::Error::custom(format!("expected #[config(...)], got {other:?}"))),
            }
        }
        None => Ok(FieldConfigArgs::default()),
    }
}
