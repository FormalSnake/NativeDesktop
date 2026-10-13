//! `#[derive(Settings)]`: collects the `#[settings(..)]` attributes of a settings struct into
//! `Settings::field_meta`. Types, defaults and doc comments come from `JsonSchema` and serde; the
//! derive only carries what they cannot express:
//!
//! * `range = 11.0..=24.0` and `step = 1.0` for numbers (slider bounds, validation),
//! * `reload = "restart"` for a field the app reads once at startup (default `"live"`),
//! * `section` for a field whose type is itself a `Settings` struct, so its attributes nest.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Expr, ExprRange, Field, Fields, Lit, LitStr, Token, parse_macro_input};

#[proc_macro_derive(Settings, attributes(settings))]
pub fn derive_settings(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input).unwrap_or_else(|e| e.to_compile_error()).into()
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(&input.ident, "#[derive(Settings)] needs a struct"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(&input.ident, "#[derive(Settings)] needs named fields"));
    };
    let rename_all = serde_rename_all(&input.attrs)?;
    let mut pushes = Vec::new();
    for field in &fields.named {
        let serde = serde_field(field)?;
        if serde.skip {
            continue;
        }
        let ident = field.ident.as_ref().expect("named field").to_string();
        let ident = ident.strip_prefix("r#").unwrap_or(&ident).to_owned();
        let key = serde.rename.unwrap_or_else(|| apply_rename(&ident, rename_all.as_deref()));
        let attrs = settings_attrs(field)?;
        let ty = &field.ty;
        if attrs.section {
            let prefix = if serde.flatten {
                quote! { m.path.clone() }
            } else {
                quote! { ::std::format!("{}.{}", #key, m.path) }
            };
            let own = if serde.flatten {
                quote! {}
            } else {
                quote! {
                    out.push(::nd_settings::FieldMeta {
                        path: ::std::string::String::from(#key),
                        range: ::core::option::Option::None,
                        step: ::core::option::Option::None,
                        reload: ::nd_settings::Reload::Live,
                    });
                }
            };
            pushes.push(quote! {
                #own
                for mut m in <#ty as ::nd_settings::Settings>::field_meta() {
                    m.path = #prefix;
                    out.push(m);
                }
            });
        } else {
            let range = match &attrs.range {
                Some((lo, hi)) => quote! { ::core::option::Option::Some((((#lo) as f64), ((#hi) as f64))) },
                None => quote! { ::core::option::Option::None },
            };
            let step = match &attrs.step {
                Some(s) => quote! { ::core::option::Option::Some(((#s) as f64)) },
                None => quote! { ::core::option::Option::None },
            };
            let reload = match attrs.reload.as_deref() {
                Some("restart") => quote! { ::nd_settings::Reload::Restart },
                _ => quote! { ::nd_settings::Reload::Live },
            };
            pushes.push(quote! {
                out.push(::nd_settings::FieldMeta {
                    path: ::std::string::String::from(#key),
                    range: #range,
                    step: #step,
                    reload: #reload,
                });
            });
        }
    }
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::nd_settings::Settings for #name #ty_generics #where_clause {
            fn field_meta() -> ::std::vec::Vec<::nd_settings::FieldMeta> {
                let mut out = ::std::vec::Vec::new();
                #(#pushes)*
                out
            }
        }
    })
}

#[derive(Default)]
struct SettingsAttrs {
    range: Option<(Expr, Expr)>,
    step: Option<Expr>,
    reload: Option<String>,
    section: bool,
}

fn settings_attrs(field: &Field) -> syn::Result<SettingsAttrs> {
    let mut out = SettingsAttrs::default();
    for attr in field.attrs.iter().filter(|a| a.path().is_ident("settings")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("range") {
                let range: ExprRange = meta.value()?.parse()?;
                match (range.start, range.end) {
                    (Some(lo), Some(hi)) => out.range = Some((*lo, *hi)),
                    _ => return Err(meta.error("range needs both ends, for example 11.0..=24.0")),
                }
            } else if meta.path.is_ident("step") {
                out.step = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("reload") {
                let value: LitStr = meta.value()?.parse()?;
                if !matches!(value.value().as_str(), "live" | "restart") {
                    return Err(syn::Error::new(value.span(), "reload is \"live\" or \"restart\""));
                }
                out.reload = Some(value.value());
            } else if meta.path.is_ident("section") {
                out.section = true;
            } else {
                return Err(meta.error("unknown settings attribute: range, step, reload or section"));
            }
            Ok(())
        })?;
    }
    Ok(out)
}

#[derive(Default)]
struct SerdeField {
    rename: Option<String>,
    skip: bool,
    flatten: bool,
}

fn serde_field(field: &Field) -> syn::Result<SerdeField> {
    let mut out = SerdeField::default();
    for attr in field.attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") && meta.input.peek(Token![=]) {
                out.rename = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("skip") {
                out.skip = true;
            } else if meta.path.is_ident("flatten") {
                out.flatten = true;
            } else {
                skip_meta(&meta)?;
            }
            Ok(())
        })?;
    }
    Ok(out)
}

fn serde_rename_all(attrs: &[syn::Attribute]) -> syn::Result<Option<String>> {
    let mut out = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") && meta.input.peek(Token![=]) {
                let lit: Lit = meta.value()?.parse()?;
                if let Lit::Str(s) = lit {
                    out = Some(s.value());
                }
            } else {
                skip_meta(&meta)?;
            }
            Ok(())
        })?;
    }
    Ok(out)
}

/// Consumes the value of a serde option this derive does not read.
fn skip_meta(meta: &syn::meta::ParseNestedMeta) -> syn::Result<()> {
    if meta.input.peek(Token![=]) {
        let _: Expr = meta.value()?.parse()?;
    } else if meta.input.peek(syn::token::Paren) {
        meta.parse_nested_meta(|inner| skip_meta(&inner))?;
    }
    Ok(())
}

fn apply_rename(ident: &str, rule: Option<&str>) -> String {
    let words: Vec<&str> = ident.split('_').filter(|w| !w.is_empty()).collect();
    let upper_first = |w: &str| {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    match rule {
        Some("kebab-case") => words.join("-"),
        Some("camelCase") => {
            let mut it = words.iter();
            let first = it.next().copied().unwrap_or("").to_owned();
            first + &it.map(|w| upper_first(w)).collect::<String>()
        }
        Some("PascalCase") => words.iter().map(|w| upper_first(w)).collect(),
        Some("UPPERCASE") => ident.to_uppercase(),
        Some("SCREAMING_SNAKE_CASE") => ident.to_uppercase(),
        Some("SCREAMING-KEBAB-CASE") => words.join("-").to_uppercase(),
        Some("lowercase") => ident.to_lowercase(),
        _ => ident.to_owned(),
    }
}
