use proc_macro::TokenStream;
use syn::parse_macro_input;
use syn::AttributeArgs;
use syn::ItemFn;

mod attachment;
mod command;
mod configuration;
mod mask;
mod message;
mod registry;

#[proc_macro_derive(Attachment, attributes(attachment))]
pub fn attachment(input: TokenStream) -> TokenStream {
    attachment::attachment(input)
}

#[proc_macro_derive(Configuration, attributes(config))]
pub fn configuration(input: TokenStream) -> TokenStream {
    configuration::configuration(input)
}

#[proc_macro_derive(Message, attributes(message))]
pub fn ygopro_message(input: TokenStream) -> TokenStream {
    message::ygopro_message(input)
}

#[proc_macro_derive(GameMessage, attributes(mask, mask_if, wait_for))]
pub fn mask(input: TokenStream) -> TokenStream {
    mask::mask(input)
}


#[proc_macro_attribute]
pub fn handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    dispatch(attr, item, |priority| priority.unwrap_or(128))
}

#[proc_macro_attribute]
pub fn before(attr: TokenStream, item: TokenStream) -> TokenStream {
    dispatch(attr, item, |priority| 128u8 - priority.unwrap_or(1))
}

#[proc_macro_attribute]
pub fn after(attr: TokenStream, item: TokenStream) -> TokenStream {
    dispatch(attr, item, |priority| 128u8 + priority.unwrap_or(1))
}

#[proc_macro_attribute]
pub fn command(attr: TokenStream, item: TokenStream) -> TokenStream {
    let _ = attr;
    let function = parse_macro_input!(item as ItemFn);
    command::command_impl(function).into()
}

fn dispatch(attr: TokenStream, item: TokenStream, transform_priority: impl Fn(Option<u8>) -> u8) -> TokenStream {
    let attr = parse_macro_input!(attr as AttributeArgs);
    let function = parse_macro_input!(item as ItemFn);
    let args = match registry::parse_args(&attr, transform_priority) {
        Ok(args) => args,
        Err(err) => return err.write_errors().into(),
    };
    registry::shared_impl(args, function).into()
}

#[proc_macro_attribute]
pub fn register_to(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr2: proc_macro2::TokenStream = attr.into();
    let info = match registry::parse_register_info(attr2) {
        Ok(info) => info,
        Err(err) => return err.to_compile_error().into(),
    };
    let function = parse_macro_input!(item as ItemFn);
    registry::register_to_impl(info, function).into()
}

