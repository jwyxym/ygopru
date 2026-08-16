use darling::FromMeta;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::ItemFn;
use syn::NestedMeta;
use syn::Path;
use syn::Ident;
use syn::Token;

#[derive(FromMeta)]
struct HandlerArgs {
    #[darling(default)]
    priority: Option<u8>,
}

pub struct ParsedArgs {
    pub key: Path,
    pub priority: u8,
}

pub fn parse_args(attr: &[NestedMeta], transform_priority: impl Fn(Option<u8>) -> u8) -> Result<ParsedArgs, darling::Error> {
    let key = attr.iter().find_map(|item| match item {
        NestedMeta::Meta(syn::Meta::Path(path)) => Some(path.clone()),
        _ => None,
    });
    let key = key.ok_or_else(|| darling::Error::custom("missing key or message"))?;
    let named: Vec<NestedMeta> = attr.iter()
        .filter(|item| !matches!(item, NestedMeta::Meta(syn::Meta::Path(_))))
        .cloned()
        .collect();
    let args = HandlerArgs::from_list(&named)?;
    let priority = transform_priority(args.priority);
    Ok(ParsedArgs { key, priority })
}

pub struct RegisterInfo {
    #[allow(dead_code)]
    pub slice_expression: syn::Path,
    pub handler_type: syn::Type,
    pub key_type: syn::Type,
}

pub fn parse_registers(function: &ItemFn) -> Vec<RegisterInfo> {
    let mut register_infos = Vec::new();
    for attr in &function.attrs {
        if attr.path.is_ident("register_to") {
            if let Ok(info) = parse_register_info(attr.tokens.clone()) {
                register_infos.push(info);
            }
        }
    }
    register_infos
}

pub fn parse_register_info(tokens: TokenStream2) -> syn::Result<RegisterInfo> {
    // `#[handler]` passes the attribute tokens wrapped in parens, while the
    // standalone `#[register_to]` macro receives them without delimiters.
    let tokens = match tokens.clone().into_iter().collect::<Vec<_>>().as_slice() {
        [proc_macro2::TokenTree::Group(group)] if group.delimiter() == proc_macro2::Delimiter::Parenthesis => group.stream(),
        _ => tokens,
    };
    syn::parse::Parser::parse2(
        |input: syn::parse::ParseStream| {
            let slice_expression = input.parse::<syn::Path>()?;

            let handler_type = if input.peek(Token![as]) {
                input.parse::<Token![as]>()?;
                input.parse::<syn::Type>()?
            } else {
                syn::parse_str::<syn::Type>("Handler")?
            };

            let key_type = if input.fork().parse::<syn::Ident>().map_or(false, |ident| ident == "with") {
                input.parse::<syn::Ident>()?;
                input.parse::<syn::Type>()?
            } else {
                syn::parse_str::<syn::Type>("u8")?
            };

            Ok(RegisterInfo { slice_expression, handler_type, key_type })
        },
        tokens,
    )
}

pub(crate) fn extract_type_suffix(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(type_path) => {
            type_path.path.segments.last()
                .map(|seg| seg.ident.to_string().to_lowercase())
                .unwrap_or_else(|| "handler".to_string())
        }
        _ => "handler".to_string(),
    }
}

pub fn shared_impl(args: ParsedArgs, function: ItemFn) -> TokenStream2 {
    let function_ident = &function.sig.ident;
    let function_name = function_ident.to_string();

    let priority = args.priority;

    let key = args.key;

    let register_infos = parse_registers(&function);

    if register_infos.is_empty() {
        let error = syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("no #[register_to] attribute found on handler `{function_name}`"),
        ).to_compile_error();
        return quote! {
            #error
            #function
        };
    }

    let registrations: Vec<_> = register_infos.iter().map(|info| {
        let suffix = extract_type_suffix(&info.handler_type);
        let builder_ident = Ident::new(
            &format!("build_handle_{function_name}_{suffix}"),
            function_ident.span(),
        );
        let key_type = &info.key_type;
        let handler_type = &info.handler_type;

        quote! {
            fn #builder_ident() -> (#key_type, #handler_type) {
                (
                    ::std::convert::Into::<#key_type>::into(<#key as ::ygopro_data::message::Message>::message_type()),
                    #handler_type::new(#priority, #function_name, module_path!(), #function_ident),
                )
            }
        }
    }).collect();

    quote! {
        #function
        #(#registrations)*
    }
}

pub fn register_to_impl(info: RegisterInfo, function: ItemFn) -> TokenStream2 {
    let function_ident = &function.sig.ident;
    let function_name = function_ident.to_string();

    let suffix = extract_type_suffix(&info.handler_type);

    let builder_ident = Ident::new(
        &format!("build_handle_{function_name}_{suffix}"),
        function_ident.span(),
    );

    let register_ident = Ident::new(
        &format!("REGISTER_{}_{}", function_ident.to_string().to_uppercase(), suffix.to_uppercase()),
        function_ident.span(),
    );

    let slice_expression = &info.slice_expression;
    let key_type = &info.key_type;
    let handler_type = &info.handler_type;

    quote! {
        #function

        #[linkme::distributed_slice(#slice_expression)]
        static #register_ident: fn() -> (#key_type, #handler_type) = #builder_ident;
    }
}
