// Phoneme bank registry, layering, and resolved-bank lookup.
//
// A bank is a JSON object (schemaVersion 1) whose `phonemes` map phoneme codes
// to parameter sets. `extends` names a parent bank to inherit from; a `null`
// entry removes an inherited phoneme. Banks can be registered at runtime from
// JSON text or files, so games can ship their own voices.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use indexmap::IndexMap;

use crate::error::{Error, Result};
use crate::value::{map_to_json, Map, Value};

pub const DEFAULT_BANK: &str = "klatt1980-en";
pub const SCHEMA_VERSION: f64 = 1.0;

// Same order as the JS bundle (sorted by file name).
const BUNDLED: [&str; 3] = [
    include_str!("../banks/ja-hecko-2026.json"),
    include_str!("../banks/ja-mokhtari-2000.json"),
    include_str!("../banks/klatt1980-en.json"),
];

// A bank with its `extends` chain flattened into `phonemes`.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedBank {
    pub schema_version: Option<Value>,
    pub name: Option<Value>,
    pub display_name: Option<Value>,
    pub language: Value,
    pub license: Value,
    pub source: Value,
    pub phonemes: Map,
}

fn non_null(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !v.is_null())
}

impl ResolvedBank {
    // A bank object used as-is (no `extends`), as resolveBank does for
    // already-flat bank objects.
    pub fn from_flat(bank: &Value) -> Self {
        let name = bank.get("name").cloned();
        Self {
            schema_version: bank.get("schemaVersion").cloned(),
            display_name: non_null(bank.get("displayName")).cloned().or_else(|| name.clone()),
            name,
            language: bank.get("language").cloned().unwrap_or_default(),
            license: bank.get("license").cloned().unwrap_or_default(),
            source: bank.get("source").cloned().unwrap_or_default(),
            phonemes: bank.get("phonemes").and_then(Value::as_obj).cloned().unwrap_or_default(),
        }
    }

    pub fn name_str(&self) -> Option<&str> {
        self.name.as_ref().and_then(Value::as_str)
    }

    pub fn display_name_str(&self) -> Option<&str> {
        self.display_name.as_ref().and_then(Value::as_str)
    }

    // The parameter set for a phoneme code, if the bank defines one.
    pub fn phoneme(&self, code: &str) -> Option<&Value> {
        self.phonemes.get(code).filter(|p| p.truthy())
    }

    // Phoneme codes, excluding internal ones that start with `_`.
    pub fn phoneme_keys(&self) -> Vec<String> {
        self.phonemes.keys().filter(|k| !k.starts_with('_')).cloned().collect()
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut o = serde_json::Map::new();
        if let Some(v) = &self.schema_version {
            o.insert("schemaVersion".into(), v.to_json());
        }
        if let Some(v) = &self.name {
            o.insert("name".into(), v.to_json());
        }
        if let Some(v) = &self.display_name {
            o.insert("displayName".into(), v.to_json());
        }
        o.insert("language".into(), self.language.to_json());
        o.insert("license".into(), self.license.to_json());
        o.insert("source".into(), self.source.to_json());
        o.insert("extends".into(), serde_json::Value::Null);
        o.insert("phonemes".into(), map_to_json(&self.phonemes));
        serde_json::Value::Object(o)
    }
}

// How a compile picks its initial bank.
#[derive(Clone, Debug, PartialEq)]
pub enum BankRef {
    // A registered bank name.
    Name(String),
    // A bank object: used directly when it has no `extends`, otherwise
    // registered (if new) and resolved by name.
    Value(Value),
    Resolved(Arc<ResolvedBank>),
}

impl From<&str> for BankRef {
    fn from(s: &str) -> Self {
        BankRef::Name(s.into())
    }
}

#[derive(Default)]
struct Inner {
    registry: IndexMap<String, Value>,
    cache: HashMap<String, Arc<ResolvedBank>>,
}

pub struct BankRegistry {
    inner: Mutex<Inner>,
}

impl Default for BankRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BankRegistry {
    // A registry preloaded with the bundled banks.
    pub fn new() -> Self {
        let reg = Self::empty();
        {
            let mut inner = reg.lock();
            for text in BUNDLED {
                let bank = Value::parse_json(text).expect("bundled bank is valid JSON");
                let name = bank.get("name").and_then(Value::as_str).expect("bundled bank name").to_string();
                inner.registry.insert(name, bank);
            }
        }
        reg
    }

    pub fn empty() -> Self {
        Self { inner: Mutex::new(Inner::default()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn default_name(&self) -> &'static str {
        DEFAULT_BANK
    }

    pub fn list(&self) -> Vec<String> {
        self.lock().registry.keys().cloned().collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.lock().registry.contains_key(name)
    }

    // The resolved bank, `Ok(None)` if no bank has this name, or an error if
    // its `extends` chain is broken or cyclic.
    pub fn get(&self, name: &str) -> Result<Option<Arc<ResolvedBank>>> {
        let mut inner = self.lock();
        if let Some(b) = inner.cache.get(name) {
            return Ok(Some(b.clone()));
        }
        if !inner.registry.contains_key(name) {
            return Ok(None);
        }
        let resolved = Arc::new(resolve_internal(&inner.registry, name, &[])?);
        inner.cache.insert(name.to_string(), resolved.clone());
        Ok(Some(resolved))
    }

    pub fn register(&self, bank: Value) -> Result<()> {
        if !matches!(bank, Value::Obj(_) | Value::Arr(_)) {
            return Err(Error::new("registerBank: bank must be an object"));
        }
        let name = match bank.get("name") {
            Some(Value::Str(s)) if !s.is_empty() => s.clone(),
            _ => return Err(Error::new("registerBank: bank.name is required")),
        };
        let version = bank.get("schemaVersion");
        if version.and_then(Value::as_f64) != Some(SCHEMA_VERSION) {
            let shown = version.map_or_else(|| "undefined".to_string(), Value::to_js_string);
            return Err(Error::new(format!(
                "registerBank: unsupported schemaVersion {shown} (expected 1)"
            )));
        }
        let mut inner = self.lock();
        inner.registry.insert(name, bank);
        inner.cache.clear();
        Ok(())
    }

    // Register a bank from JSON text; returns its name.
    pub fn register_json(&self, json: &str) -> Result<String> {
        let bank = Value::parse_json(json)?;
        let name = bank.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        self.register(bank)?;
        Ok(name)
    }

    // Register a bank from a JSON file; returns its name.
    pub fn load_file(&self, path: impl AsRef<Path>) -> Result<String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::new(format!("cannot read bank {}: {e}", path.display())))?;
        self.register_json(&text)
    }

    // resolveBank: `None` picks the default bank.
    pub fn resolve(&self, bank: Option<&BankRef>) -> Result<Arc<ResolvedBank>> {
        let unknown = |name: &str| Error::new(format!("unknown bank: {name}"));
        match bank {
            None => self.get(DEFAULT_BANK)?.ok_or_else(|| unknown(DEFAULT_BANK)),
            Some(BankRef::Name(name)) => self.get(name)?.ok_or_else(|| unknown(name)),
            Some(BankRef::Resolved(b)) => Ok(b.clone()),
            Some(BankRef::Value(v)) => match v {
                Value::Null => self.resolve(None),
                Value::Str(name) => self.get(name)?.ok_or_else(|| unknown(name)),
                Value::Obj(_) | Value::Arr(_) => {
                    let extends_null = v.get("extends").is_none_or(Value::is_null);
                    if extends_null && v.get("phonemes").is_some_and(Value::truthy) {
                        return Ok(Arc::new(ResolvedBank::from_flat(v)));
                    }
                    let name = v.get("name").filter(|n| n.truthy());
                    if let Some(name) = name {
                        if let Some(n) = name.as_str().filter(|n| self.contains(n)) {
                            return self.get(n)?.ok_or_else(|| unknown(n));
                        }
                        self.register(v.clone())?;
                        let n = name.as_str().unwrap_or_default();
                        return self.get(n)?.ok_or_else(|| unknown(n));
                    }
                    Err(Error::new("resolveBank: unsupported bank value"))
                }
                _ => Err(Error::new("resolveBank: unsupported bank value")),
            },
        }
    }
}

fn resolve_internal(registry: &IndexMap<String, Value>, name: &str, visiting: &[String]) -> Result<ResolvedBank> {
    if visiting.iter().any(|v| v == name) {
        let mut path: Vec<&str> = visiting.iter().map(String::as_str).collect();
        path.push(name);
        return Err(Error::new(format!("bank extends cycle detected: {}", path.join(" -> "))));
    }
    let bank = registry.get(name).ok_or_else(|| Error::new(format!("unknown bank: {name}")))?;

    let mut phonemes = Map::new();
    if let Some(parent) = bank.get("extends").filter(|e| e.truthy()) {
        let mut chain = visiting.to_vec();
        chain.push(name.to_string());
        let parent_name = parent.to_js_string();
        let resolved = match parent {
            Value::Str(p) => resolve_internal(registry, p, &chain)?,
            _ => return Err(Error::new(format!("unknown bank: {parent_name}"))),
        };
        phonemes = resolved.phonemes;
    }

    if let Some(entries) = bank.get("phonemes").and_then(Value::as_obj) {
        for (code, entry) in entries {
            if entry.is_null() {
                phonemes.shift_remove(code);
            } else {
                phonemes.insert(code.clone(), entry.clone());
            }
        }
    }

    let bank_name = bank.get("name").cloned();
    Ok(ResolvedBank {
        schema_version: bank.get("schemaVersion").cloned(),
        display_name: non_null(bank.get("displayName")).cloned().or_else(|| bank_name.clone()),
        name: bank_name,
        language: non_null(bank.get("language")).cloned().unwrap_or_default(),
        license: non_null(bank.get("license")).cloned().unwrap_or_default(),
        source: non_null(bank.get("source")).cloned().unwrap_or_default(),
        phonemes,
    })
}

static DEFAULT_REGISTRY: LazyLock<BankRegistry> = LazyLock::new(BankRegistry::new);

// The process-wide registry, like the JS module-level `banks`.
pub fn banks() -> &'static BankRegistry {
    &DEFAULT_REGISTRY
}

pub fn register_bank(bank: Value) -> Result<()> {
    banks().register(bank)
}

pub fn resolve_bank(bank: Option<&BankRef>) -> Result<Arc<ResolvedBank>> {
    banks().resolve(bank)
}

// Parameter sets of the default English bank (the legacy `phonemes` export).
pub fn phonemes() -> Arc<ResolvedBank> {
    banks().get(DEFAULT_BANK).ok().flatten().expect("default bank is bundled")
}

// Codes of the default bank, excluding internal ones (`PHONEME_KEYS`).
pub fn phoneme_keys() -> Vec<String> {
    phonemes().phoneme_keys()
}
