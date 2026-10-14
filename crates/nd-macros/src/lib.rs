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

/// Marks an API that cannot work on the listed targets (`#[nd::unavailable(web)]`). The item is
/// unchanged; `nd check --target web` reads the marker and reports each call site that is not
/// guarded by `nd::platform::has(..)`, and the docs gain a line naming the targets.
#[proc_macro_attribute]
pub fn unavailable(attr: TokenStream, item: TokenStream) -> TokenStream {
    let targets: Vec<String> = attr
        .into_iter()
        .filter_map(|t| match t {
            TokenTree::Ident(i) => Some(i.to_string()),
            _ => None,
        })
        .collect();
    if targets.is_empty() || targets.iter().any(|t| !matches!(t.as_str(), "web" | "macos" | "windows" | "linux")) {
        return error("#[unavailable(..)] takes targets: web, macos, windows, linux");
    }
    let doc = format!("\n\nUnavailable on: {}. Check `nd::platform::has(..)` first.", targets.join(", "));
    let mut out: TokenStream = format!("#[doc = {doc:?}]").parse().expect("doc attribute parses");
    out.extend(item);
    out
}

/// A tier 1 test that runs natively under libtest and in a browser under `wasm-bindgen-test`
/// (`--target wasm32-unknown-unknown`). Takes `fn name()` or `fn name(cx: &mut TestApp)`, which
/// gets a fresh `nd::test::TestApp`.
#[proc_macro_attribute]
pub fn test(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return error("#[nd::test] takes no arguments");
    }
    let tokens: Vec<TokenTree> = item.into_iter().collect();
    if tokens.iter().any(|t| matches!(t, TokenTree::Ident(i) if i.to_string() == "async")) {
        return error("#[nd::test] functions are synchronous; tier 1 settles with `run_until_idle`");
    }
    let Some(fn_at) = tokens.iter().position(|t| matches!(t, TokenTree::Ident(i) if i.to_string() == "fn")) else {
        return error("#[nd::test] goes on a function");
    };
    let (Some(TokenTree::Ident(name)), Some(TokenTree::Group(params))) = (tokens.get(fn_at + 1), tokens.get(fn_at + 2))
    else {
        return error("#[nd::test] needs `fn name(..)`");
    };
    let Some(TokenTree::Group(body)) = tokens.last() else { return error("#[nd::test] needs a body") };
    let ret: TokenStream = tokens[fn_at + 3..tokens.len() - 1].iter().cloned().collect();
    let before: TokenStream = tokens[..fn_at].iter().cloned().collect();
    let attrs = "#[cfg_attr(target_arch = \"wasm32\", ::nd::__private::wasm_bindgen_test::wasm_bindgen_test(crate = ::nd::__private::wasm_bindgen_test))] \
                 #[cfg_attr(not(target_arch = \"wasm32\"), test)]";
    let source = if params.stream().is_empty() {
        format!("{attrs} {before} fn {name}() {ret} {body}")
    } else {
        format!(
            "{attrs} {before} fn {name}() {ret} {{ fn inner({}) {ret} {body} inner(&mut ::nd::test::TestApp::new()) }}",
            params.stream()
        )
    };
    source.parse().unwrap_or_else(|_| error("#[nd::test] could not rebuild the function"))
}
