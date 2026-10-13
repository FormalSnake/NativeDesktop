//! nd's derives (ARCHITECTURE.md 2). Apps reach them through `nd::prelude`.

use proc_macro::{TokenStream, TokenTree};

/// Mounts a native element: implements `IntoElement` for a type that implements
/// `nd::native::NativeElement`. Generic element types are not supported yet.
#[proc_macro_derive(IntoElement)]
pub fn derive_into_element(input: TokenStream) -> TokenStream {
    let mut tokens = input.into_iter();
    let name = loop {
        match tokens.next() {
            Some(TokenTree::Ident(i)) if i.to_string() == "struct" || i.to_string() == "enum" => match tokens.next() {
                Some(TokenTree::Ident(name)) => break name,
                _ => return error("expected a type name"),
            },
            Some(_) => continue,
            None => return error("#[derive(IntoElement)] needs a struct or enum"),
        }
    };
    if matches!(tokens.next(), Some(TokenTree::Punct(p)) if p.as_char() == '<') {
        return error("#[derive(IntoElement)] does not support generic types yet");
    }
    format!(
        "impl ::nd::prelude::IntoElement for {name} {{
            fn into_any_element(self) -> ::nd::prelude::AnyElement {{
                ::nd::native::into_any_element(self)
            }}
        }}"
    )
    .parse()
    .expect("generated impl parses")
}

fn error(message: &str) -> TokenStream {
    format!("::core::compile_error!({message:?});").parse().expect("compile_error parses")
}
