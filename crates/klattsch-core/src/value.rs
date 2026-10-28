// A JSON-like dynamic value with JavaScript coercion rules.
//
// Phoneme banks and schedule targets are open-ended objects in the JS engine:
// banks can carry extra fields (`ipa`, `example`, `F4`, ...) that ride along
// into schedule targets for other engines to read. `Value` keeps that data
// intact, preserves key order, and can hold NaN, which JSON cannot.

use indexmap::IndexMap;

use crate::error::{Error, Result};
use crate::js;

pub type Map = IndexMap<String, Value>;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Map),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(m) => m.get(key),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&Map> {
        match self {
            Value::Obj(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    // JS truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Num(n) => !(*n == 0.0 || n.is_nan()),
            Value::Str(s) => !s.is_empty(),
            Value::Arr(_) | Value::Obj(_) => true,
        }
    }

    // The own properties `{ ...value }` copies: an object's entries, a
    // string's UTF-16 code units or an array's elements under index keys, and
    // nothing for other primitives. (A lone surrogate becomes U+FFFD, since a
    // Rust string cannot hold one.)
    pub fn spread(&self) -> Map {
        match self {
            Value::Obj(m) => m.clone(),
            Value::Str(s) => s
                .encode_utf16()
                .enumerate()
                .map(|(i, u)| (i.to_string(), Value::Str(String::from_utf16_lossy(&[u]))))
                .collect(),
            Value::Arr(a) => a.iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect(),
            _ => Map::new(),
        }
    }

    // JS ToNumber.
    pub fn to_number(&self) -> f64 {
        match self {
            Value::Null => 0.0,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Num(n) => *n,
            Value::Str(s) => js::string_to_number(s),
            // Arrays convert through their string form: [] -> "" -> 0,
            // [null] -> "" -> 0, [[7]] -> "7" -> 7, [1, 2] -> "1,2" -> NaN.
            Value::Arr(_) => js::string_to_number(&self.to_js_string()),
            Value::Obj(_) => f64::NAN,
        }
    }

    // JS String(value).
    pub fn to_js_string(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => js::number_to_string(*n),
            Value::Str(s) => s.clone(),
            Value::Arr(a) => a
                .iter()
                .map(|v| if v.is_null() { String::new() } else { v.to_js_string() })
                .collect::<Vec<_>>()
                .join(","),
            Value::Obj(_) => "[object Object]".into(),
        }
    }

    pub fn parse_json(text: &str) -> Result<Value> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| Error::new(format!("invalid JSON: {e}")))?;
        Ok(Value::from_json(&v))
    }

    pub fn from_json(v: &serde_json::Value) -> Value {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => Value::Num(n.as_f64().unwrap_or(f64::NAN)),
            serde_json::Value::String(s) => Value::Str(s.clone()),
            serde_json::Value::Array(a) => Value::Arr(a.iter().map(Value::from_json).collect()),
            serde_json::Value::Object(o) => {
                Value::Obj(o.iter().map(|(k, v)| (k.clone(), Value::from_json(v))).collect())
            }
        }
    }

    // Like JSON.stringify: non-finite numbers become null.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Num(n) => num_json(*n),
            Value::Str(s) => serde_json::Value::String(s.clone()),
            Value::Arr(a) => serde_json::Value::Array(a.iter().map(Value::to_json).collect()),
            Value::Obj(m) => map_to_json(m),
        }
    }
}

pub fn map_to_json(m: &Map) -> serde_json::Value {
    serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
}

pub(crate) fn num_json(n: f64) -> serde_json::Value {
    serde_json::Number::from_f64(n).map_or(serde_json::Value::Null, serde_json::Value::Number)
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Num(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.into())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}

impl From<Map> for Value {
    fn from(m: Map) -> Self {
        Value::Obj(m)
    }
}
