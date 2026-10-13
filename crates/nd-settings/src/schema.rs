//! The JSON Schema of a settings struct, and the typed view of it that the settings form and the
//! home-manager generator both read.

use schemars::JsonSchema;
use serde_json::{Map, Value, json};

use crate::{FieldMeta, Reload, Settings};

/// The schema schemars derives for `T`, with the `#[settings(..)]` data merged in: `minimum` and
/// `maximum` from `range`, `x-nd-step`, `x-nd-reload`, and `x-nd-order` (declaration order, which
/// the JSON object does not keep).
pub fn schema_json<T: Settings>() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes");
    for (order, meta) in T::field_meta().into_iter().enumerate() {
        let Some(pointer) = property_pointer(&schema, &meta.path) else { continue };
        let Some(Value::Object(node)) = schema.pointer_mut(&pointer) else { continue };
        node.insert("x-nd-order".into(), json!(order));
        if let Some((lo, hi)) = meta.range {
            node.insert("minimum".into(), number(lo));
            node.insert("maximum".into(), number(hi));
        }
        if let Some(step) = meta.step {
            node.insert("x-nd-step".into(), json!(step));
        }
        if meta.reload == Reload::Restart {
            node.insert("x-nd-reload".into(), json!("restart"));
        }
    }
    schema
}

/// Integral floats print as integers, so `11.0` is `11` in the schema and in Nix.
fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 { json!(v as i64) } else { json!(v) }
}

fn ref_pointer(node: &Value) -> Option<String> {
    node.get("$ref")?.as_str()?.strip_prefix('#').map(str::to_owned)
}

/// JSON pointer to the schema node of the property at the dotted `path`.
fn property_pointer(root: &Value, path: &str) -> Option<String> {
    let mut pointer = String::new();
    for segment in path.split('.') {
        let mut node = root.pointer(&pointer)?;
        while let Some(target) = ref_pointer(node) {
            pointer = target;
            node = root.pointer(&pointer)?;
        }
        node.get("properties")?.get(segment)?;
        pointer = format!("{pointer}/properties/{}", segment.replace('~', "~0").replace('/', "~1"));
    }
    Some(pointer)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub value: String,
    pub help: Option<String>,
}

/// A property's type, with `$ref`s resolved.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ty {
    Bool,
    Int {
        min: Option<f64>,
        max: Option<f64>,
    },
    Number {
        min: Option<f64>,
        max: Option<f64>,
    },
    Str,
    Enum(Vec<Choice>),
    List(Box<Ty>),
    Map(Box<Ty>),
    Object(Vec<Prop>),
    Nullable(Box<Ty>),
    /// Anything the walker does not model: tagged unions, free-form values.
    Any,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Prop {
    pub name: String,
    pub doc: Option<String>,
    pub ty: Ty,
    pub default: Option<Value>,
    pub step: Option<f64>,
    pub reload: Reload,
}

/// The overlay of a node's `$ref` target and its own keys, which win.
fn resolve(node: &Value, root: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(target) = ref_pointer(node).and_then(|p| root.pointer(&p)) {
        out = resolve(target, root);
    }
    if let Value::Object(own) = node {
        for (k, v) in own {
            if k != "$ref" {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

fn doc_of(node: &Map<String, Value>) -> Option<String> {
    node.get("description").and_then(Value::as_str).map(|d| d.trim().to_owned()).filter(|d| !d.is_empty())
}

pub(crate) fn ty_of(node: &Value, root: &Value) -> Ty {
    let node = resolve(node, root);
    for key in ["oneOf", "anyOf"] {
        let Some(Value::Array(options)) = node.get(key) else { continue };
        let (null, rest): (Vec<_>, Vec<_>) = options.iter().partition(|o| o.get("type") == Some(&json!("null")));
        if !null.is_empty() && rest.len() == 1 {
            return Ty::Nullable(Box::new(ty_of(rest[0], root)));
        }
        let choices: Option<Vec<Choice>> = options
            .iter()
            .map(|o| {
                let o = resolve(o, root);
                let value = o.get("const").and_then(Value::as_str).map(str::to_owned).or_else(|| {
                    match o.get("enum").and_then(Value::as_array).map(Vec::as_slice) {
                        Some([Value::String(only)]) => Some(only.clone()),
                        _ => None,
                    }
                })?;
                Some(Choice { value, help: doc_of(&o) })
            })
            .collect();
        return choices.map_or(Ty::Any, Ty::Enum);
    }
    if let Some(Value::Array(values)) = node.get("enum") {
        let choices: Option<Vec<Choice>> =
            values.iter().map(|v| v.as_str().map(|s| Choice { value: s.to_owned(), help: None })).collect();
        return choices.map_or(Ty::Any, Ty::Enum);
    }
    let min = node.get("minimum").and_then(Value::as_f64);
    let max = node.get("maximum").and_then(Value::as_f64);
    let kind = |name: &str| -> Ty {
        match name {
            "boolean" => Ty::Bool,
            "integer" => Ty::Int { min, max },
            "number" => Ty::Number { min, max },
            "string" => Ty::Str,
            "array" => node.get("items").map_or(Ty::Any, |i| Ty::List(Box::new(ty_of(i, root)))),
            "object" => object_ty(&node, root),
            _ => Ty::Any,
        }
    };
    match node.get("type") {
        Some(Value::String(name)) => kind(name),
        Some(Value::Array(names)) => {
            let names: Vec<&str> = names.iter().filter_map(Value::as_str).collect();
            let base: Vec<&&str> = names.iter().filter(|n| **n != "null").collect();
            match (base.as_slice(), names.contains(&"null")) {
                ([only], true) => Ty::Nullable(Box::new(kind(only))),
                ([only], false) => kind(only),
                _ => Ty::Any,
            }
        }
        _ => Ty::Any,
    }
}

fn object_ty(node: &Map<String, Value>, root: &Value) -> Ty {
    if let Some(Value::Object(props)) = node.get("properties") {
        let mut out: Vec<(usize, Prop)> = props
            .iter()
            .enumerate()
            .map(|(i, (name, schema))| {
                let resolved = resolve(schema, root);
                let order =
                    resolved.get("x-nd-order").and_then(Value::as_u64).map_or(usize::MAX - 1 - i, |o| o as usize);
                let prop = Prop {
                    name: name.clone(),
                    doc: doc_of(&resolved),
                    ty: ty_of(schema, root),
                    default: resolved.get("default").cloned(),
                    step: resolved.get("x-nd-step").and_then(Value::as_f64),
                    reload: if resolved.get("x-nd-reload") == Some(&json!("restart")) {
                        Reload::Restart
                    } else {
                        Reload::Live
                    },
                };
                (order, prop)
            })
            .collect();
        out.sort_by_key(|(order, _)| *order);
        return Ty::Object(out.into_iter().map(|(_, p)| p).collect());
    }
    match node.get("additionalProperties") {
        Some(inner @ Value::Object(_)) => Ty::Map(Box::new(ty_of(inner, root))),
        _ => Ty::Any,
    }
}

/// The root object's properties.
pub(crate) fn root_props(root: &Value) -> Vec<Prop> {
    match ty_of(root, root) {
        Ty::Object(props) => props,
        _ => Vec::new(),
    }
}

/// How the settings form renders one key.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Toggle,
    Integer {
        min: Option<f64>,
        max: Option<f64>,
    },
    Number {
        min: Option<f64>,
        max: Option<f64>,
    },
    Text,
    Choice(Vec<Choice>),
    List(Box<FieldKind>),
    /// A table of free keys; edited as raw TOML.
    Map,
    /// A shape the form does not model; edited as raw TOML.
    Raw,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// Dotted key, as written in `config.toml`.
    pub path: String,
    pub title: String,
    /// The field's doc comment.
    pub help: Option<String>,
    pub kind: FieldKind,
    /// `Option<T>` fields: unset is a valid state.
    pub optional: bool,
    pub default: Option<Value>,
    pub step: Option<f64>,
    pub reload: Reload,
}

/// A nested settings struct; its fields follow in [`SettingsSchema::fields`] with the path as prefix.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionInfo {
    pub path: String,
    pub title: String,
    pub help: Option<String>,
}

/// Everything `settings_form::<T>()` needs: rows in declaration order, grouped by section.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsSchema {
    pub sections: Vec<SectionInfo>,
    pub fields: Vec<Field>,
    meta: Vec<FieldMeta>,
}

impl SettingsSchema {
    pub fn of<T: Settings + JsonSchema>() -> Self {
        let root = schema_json::<T>();
        let mut out = Self { sections: Vec::new(), fields: Vec::new(), meta: T::field_meta() };
        out.collect("", &root_props(&root));
        out
    }

    /// Adds one field per key of the plugins' default sections (`plugins.<name>.<key>`), typed
    /// from the default value, under a section named after the plugin.
    pub fn with_plugins(mut self, defaults: &toml::Table) -> Self {
        for (name, section) in defaults {
            let Some(section) = section.as_table() else { continue };
            self.sections.push(SectionInfo { path: format!("plugins.{name}"), title: humanize(name), help: None });
            self.collect_plugin(&format!("plugins.{name}"), section);
        }
        self
    }

    fn collect_plugin(&mut self, prefix: &str, table: &toml::Table) {
        for (key, value) in table {
            let path = format!("{prefix}.{key}");
            if let Some(inner) = value.as_table() {
                self.collect_plugin(&path, inner);
                continue;
            }
            self.fields.push(Field {
                title: humanize(key),
                help: None,
                kind: plugin_kind(value),
                optional: false,
                default: serde_json::to_value(value).ok(),
                step: None,
                reload: Reload::Live,
                path,
            });
        }
    }

    pub fn field(&self, path: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.path == path)
    }

    pub(crate) fn meta(&self, path: &str) -> Option<&FieldMeta> {
        self.meta.iter().find(|m| m.path == path)
    }

    pub(crate) fn all_meta(&self) -> &[FieldMeta] {
        &self.meta
    }

    fn collect(&mut self, prefix: &str, props: &[Prop]) {
        for prop in props {
            let path = if prefix.is_empty() { prop.name.clone() } else { format!("{prefix}.{}", prop.name) };
            if let Ty::Object(children) = &prop.ty {
                self.sections.push(SectionInfo {
                    path: path.clone(),
                    title: humanize(&prop.name),
                    help: prop.doc.clone(),
                });
                self.collect(&path, children);
                continue;
            }
            let (ty, optional) = match &prop.ty {
                Ty::Nullable(inner) => (inner.as_ref(), true),
                other => (other, false),
            };
            let meta = self.meta(&path);
            self.fields.push(Field {
                title: humanize(&prop.name),
                help: prop.doc.clone(),
                kind: kind_of(ty),
                optional,
                default: prop.default.clone(),
                step: prop.step,
                reload: meta.map_or(prop.reload, |m| m.reload),
                path,
            });
        }
    }
}

fn kind_of(ty: &Ty) -> FieldKind {
    match ty {
        Ty::Bool => FieldKind::Toggle,
        Ty::Int { min, max } => FieldKind::Integer { min: *min, max: *max },
        Ty::Number { min, max } => FieldKind::Number { min: *min, max: *max },
        Ty::Str => FieldKind::Text,
        Ty::Enum(choices) => FieldKind::Choice(choices.clone()),
        Ty::List(inner) => FieldKind::List(Box::new(kind_of(inner))),
        Ty::Map(_) => FieldKind::Map,
        Ty::Nullable(inner) => kind_of(inner),
        Ty::Object(_) | Ty::Any => FieldKind::Raw,
    }
}

fn plugin_kind(value: &toml::Value) -> FieldKind {
    match value {
        toml::Value::Boolean(_) => FieldKind::Toggle,
        toml::Value::Integer(_) => FieldKind::Integer { min: None, max: None },
        toml::Value::Float(_) => FieldKind::Number { min: None, max: None },
        toml::Value::String(_) => FieldKind::Text,
        toml::Value::Array(items) => FieldKind::List(Box::new(items.first().map_or(FieldKind::Raw, plugin_kind))),
        _ => FieldKind::Raw,
    }
}

fn humanize(name: &str) -> String {
    let spaced = name.replace(['_', '-'], " ");
    let mut chars = spaced.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}
