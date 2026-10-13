use std::sync::Arc;

use rusqlite::types::{ToSql, ToSqlOutput, ValueRef};
use serde::de::DeserializeOwned;
use serde_json::{Map, Number};

use crate::Result;

/// One SQLite value, owned so it can cross to the database thread and back.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub(crate) fn from_ref(v: ValueRef<'_>) -> Self {
        match v {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(i) => Value::Integer(i),
            ValueRef::Real(f) => Value::Real(f),
            ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
            ValueRef::Blob(b) => Value::Blob(b.to_vec()),
        }
    }

    /// Blobs become an array of byte values; NaN and infinity become null.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Integer(i) => (*i).into(),
            Value::Real(f) => Number::from_f64(*f).map_or(serde_json::Value::Null, Into::into),
            Value::Text(s) => s.clone().into(),
            Value::Blob(b) => b.iter().map(|byte| serde_json::Value::from(*byte)).collect(),
        }
    }
}

impl ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self {
            Value::Null => ValueRef::Null,
            Value::Integer(i) => ValueRef::Integer(*i),
            Value::Real(f) => ValueRef::Real(*f),
            Value::Text(s) => ValueRef::Text(s.as_bytes()),
            Value::Blob(b) => ValueRef::Blob(b),
        }))
    }
}

macro_rules! from_int {
    ($($t:ty),*) => {$(impl From<$t> for Value { fn from(v: $t) -> Self { Value::Integer(i64::from(v)) } })*};
}
from_int!(i8, i16, i32, i64, u8, u16, u32, bool);

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Real(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_owned())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}
impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Blob(v)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

/// Builds a `Vec<Value>` of parameters: `params![1, "a", None::<i64>]`.
#[macro_export]
macro_rules! params {
    ($($v:expr),* $(,)?) => { ::std::vec![$($crate::Value::from($v)),*] };
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub(crate) columns: Arc<[String]>,
    pub(crate) values: Vec<Value>,
}

impl Row {
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// The first column with this name. Joins that repeat a name need an `AS` alias.
    pub fn get(&self, column: &str) -> Option<&Value> {
        self.columns.iter().position(|c| c == column).map(|i| &self.values[i])
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut map = Map::new();
        for (name, value) in self.columns.iter().zip(&self.values) {
            map.entry(name.clone()).or_insert_with(|| value.to_json());
        }
        serde_json::Value::Object(map)
    }

    /// Decodes the row into a struct by column name.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_json::from_value(self.to_json())?)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunResult {
    pub changes: u64,
    pub last_insert_rowid: i64,
}

/// SQL plus positional parameters (`?1` / `?`).
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<Value>,
}

impl Statement {
    pub fn new(sql: impl Into<String>, params: Vec<Value>) -> Self {
        Self { sql: sql.into(), params }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StepResult {
    /// The statement returned no columns.
    Run(RunResult),
    Rows(Vec<Row>),
}
