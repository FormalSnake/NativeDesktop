//! The component reference, generated from `nd-components`' source so it cannot drift: every
//! free `pub fn` that returns one of the crate's element types (`button`, `toolbar`, `list`..),
//! its doc comment, and the builder methods of the type it returns, grouped by module.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};
use quote::ToTokens;
use syn::{FnArg, ImplItem, Item, ReturnType, Type, Visibility};

/// Modules that hold models and helpers rather than elements.
const SKIP: &[&str] = &["fuzzy", "services", "media/pipeline", "media/clock", "panes/model", "overrides"];

#[derive(Debug, Default)]
struct Constructor {
    name: String,
    signature: String,
    doc: String,
    returns: String,
}

#[derive(Debug, Default)]
struct Method {
    name: String,
    args: String,
    summary: String,
}

fn doc_of(attrs: &[syn::Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc")
            && let syn::Meta::NameValue(nv) = &attr.meta
            && let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = &nv.value
        {
            lines.push(s.value().strip_prefix(' ').map(str::to_owned).unwrap_or_else(|| s.value()));
        }
    }
    unlink_rustdoc(lines.join("\n").trim())
}

/// Rustdoc's `[label](path::Item)` links point at items, not pages: keep the label only.
fn unlink_rustdoc(doc: &str) -> String {
    let mut out = String::new();
    let mut rest = doc;
    while let Some(start) = rest.find("](") {
        let Some(end) = rest[start + 2..].find(')') else { break };
        let target = &rest[start + 2..start + 2 + end];
        out.push_str(&rest[..start + 1]);
        if target.contains("://") {
            out.push_str(&rest[start + 1..start + 3 + end]);
        } else if let Some(open) = out.rfind('[') {
            out.remove(open);
            out.pop();
        }
        rest = &rest[start + 3 + end..];
    }
    out.push_str(rest);
    out
}

fn first_paragraph(doc: &str) -> String {
    doc.split("\n\n").next().unwrap_or("").replace('\n', " ").trim().to_owned()
}

fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

fn tokens(t: impl ToTokens) -> String {
    let text = t.to_token_stream().to_string();
    // quote spaces every token; undo the common cases so signatures read like source.
    text.replace(" :: ", "::")
        .replace(" < ", "<")
        .replace(" <", "<")
        .replace("< ", "<")
        .replace(" >", ">")
        .replace(" ,", ",")
        .replace("& ", "&")
        .replace(" (", "(")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace("' ", "'")
}

fn module_of(root: &Path, file: &Path) -> String {
    let relative = file.strip_prefix(root).unwrap_or(file).with_extension("");
    let text = relative.to_string_lossy().replace('\\', "/");
    text.strip_suffix("/mod").map(str::to_owned).unwrap_or(text)
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Markdown for the reference page, from `crates/nd-components/src` under `repo`.
pub fn reference(repo: &Path) -> Result<String> {
    let root = repo.join("crates/nd-components/src");
    let mut files = Vec::new();
    rust_files(&root, &mut files)?;
    files.sort();
    let mut constructors: BTreeMap<String, Vec<Constructor>> = BTreeMap::new();
    let mut methods: BTreeMap<String, Vec<Method>> = BTreeMap::new();
    for file in &files {
        let module = module_of(&root, file);
        if module == "lib" || SKIP.contains(&module.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(file)?;
        let parsed = syn::parse_file(&text).with_context(|| format!("parsing {}", file.display()))?;
        for item in parsed.items {
            match item {
                Item::Fn(f) if matches!(f.vis, Visibility::Public(_)) => {
                    let ReturnType::Type(_, ty) = &f.sig.output else { continue };
                    let Some(returns) = type_name(ty) else { continue };
                    let name = f.sig.ident.to_string();
                    if !returns.starts_with(char::is_uppercase) || name.starts_with(char::is_uppercase) {
                        continue;
                    }
                    let args = f.sig.inputs.iter().map(tokens).collect::<Vec<_>>().join(", ");
                    let generics =
                        if f.sig.generics.params.is_empty() { String::new() } else { tokens(&f.sig.generics) };
                    constructors.entry(module.clone()).or_default().push(Constructor {
                        signature: format!("{name}{generics}({args}) -> {}", tokens(ty)),
                        name,
                        doc: doc_of(&f.attrs),
                        returns,
                    });
                }
                Item::Impl(imp) if imp.trait_.is_none() => {
                    let Some(owner) = type_name(&imp.self_ty) else { continue };
                    for item in imp.items {
                        let ImplItem::Fn(f) = item else { continue };
                        if !matches!(f.vis, Visibility::Public(_)) {
                            continue;
                        }
                        let builder = matches!(f.sig.inputs.first(), Some(FnArg::Receiver(r)) if r.reference.is_none())
                            && matches!(&f.sig.output, ReturnType::Type(_, t) if tokens(t) == "Self");
                        if !builder {
                            continue;
                        }
                        let args = f.sig.inputs.iter().skip(1).map(tokens).collect::<Vec<_>>().join(", ");
                        methods.entry(owner.clone()).or_default().push(Method {
                            name: f.sig.ident.to_string(),
                            args,
                            summary: first_paragraph(&doc_of(&f.attrs)),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = String::new();
    let total: usize = constructors.values().map(Vec::len).sum();
    let _ = writeln!(
        out,
        "Generated from `crates/nd-components/src` by `nd-docs`: {total} element constructors in {} modules. \
         Each lists the builder methods of the element it returns.\n",
        constructors.len()
    );
    for (module, list) in &constructors {
        let _ = writeln!(out, "## `{module}`\n");
        for c in list {
            let _ = writeln!(out, "### `{}`\n", c.name);
            let _ = writeln!(out, "```rust\npub fn {}\n```\n", c.signature);
            if !c.doc.is_empty() {
                let _ = writeln!(out, "{}\n", c.doc);
            }
            if let Some(ms) = methods.get(&c.returns).filter(|m| !m.is_empty()) {
                let _ = writeln!(out, "| `{}` method | Does |\n| --- | --- |", c.returns);
                for m in ms {
                    let summary = if m.summary.is_empty() { "".to_owned() } else { m.summary.replace('|', "\\|") };
                    let _ = writeln!(out, "| `.{}({})` | {} |", m.name, m.args.replace('|', "\\|"), summary);
                }
                out.push('\n');
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn rustdoc_links_lose_their_target() {
        assert_eq!(
            super::unlink_rustdoc("See [`Foo`](crate::Foo) and [site](https://x.y)."),
            "See `Foo` and [site](https://x.y)."
        );
    }

    #[test]
    fn reads_the_real_crate() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let text = super::reference(&repo).unwrap();
        assert!(text.contains("### `toolbar`"), "{text}");
        assert!(text.contains("### `button`") || text.contains("### `text_field`"));
        assert!(!text.contains("### `rank`"), "fuzzy is skipped");
    }
}
