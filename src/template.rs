//! A template is one push written as an FCM v1 `message`, with `{{name}}` placeholders.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    #[serde(default)]
    pub description: Option<String>,
    /// Default values for the placeholders; `--set name=value` overrides them.
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    pub message: Value,
}

impl Template {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        serde_saphyr::from_str(text).map_err(|e| e.to_string())
    }

    /// The message with every placeholder filled in.
    pub fn render(&self, sets: &[(String, String)]) -> Result<Value, String> {
        fill_all(&self.message, &self.vars(sets))
    }

    /// The message with what can be filled, and the names that had no value.
    pub fn render_partial(&self, sets: &[(String, String)]) -> (Value, Vec<String>) {
        let mut missing = Vec::new();
        let message = fill(&self.message, &self.vars(sets), &mut missing);
        missing.sort();
        missing.dedup();
        (message, missing)
    }

    fn vars(&self, sets: &[(String, String)]) -> BTreeMap<String, Value> {
        let mut vars = self.params.clone();
        for (k, v) in sets {
            vars.insert(k.clone(), scalar(v));
        }
        vars
    }
}

/// `--set badge=3` gives a number, `--set name=Ada` a string.
fn scalar(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn lookup(key: &str, vars: &BTreeMap<String, Value>) -> Option<Value> {
    match key {
        "uuid" => Some(uuid::Uuid::new_v4().to_string().into()),
        "unix" => Some(now().into()),
        _ => match key.strip_prefix("env.") {
            Some(name) => std::env::var(name).ok().map(Value::String),
            None => vars.get(key).cloned(),
        },
    }
}

/// Fills placeholders everywhere in `value`, failing with every name that has no value.
pub fn fill_all(value: &Value, vars: &BTreeMap<String, Value>) -> Result<Value, String> {
    let mut missing = Vec::new();
    let out = fill(value, vars, &mut missing);
    if missing.is_empty() {
        return Ok(out);
    }
    missing.sort();
    missing.dedup();
    Err(format!(
        "no value for {}: add it under `params:`, pass --set NAME=VALUE, or set the environment variable",
        missing.join(", ")
    ))
}

fn fill(value: &Value, vars: &BTreeMap<String, Value>, missing: &mut Vec<String>) -> Value {
    match value {
        Value::String(s) => fill_str(s, vars, missing),
        Value::Array(a) => a.iter().map(|v| fill(v, vars, missing)).collect(),
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| (k.clone(), fill(v, vars, missing)))
            .collect(),
        other => other.clone(),
    }
}

fn fill_str(s: &str, vars: &BTreeMap<String, Value>, missing: &mut Vec<String>) -> Value {
    // a string that is exactly one placeholder takes the value's own type
    if let Some(key) = s.strip_prefix("{{").and_then(|r| r.strip_suffix("}}"))
        && !key.contains("{{")
        && !key.contains("}}")
    {
        return lookup(key.trim(), vars).unwrap_or_else(|| {
            missing.push(key.trim().to_string());
            Value::String(s.to_string())
        });
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else {
            break;
        };
        out.push_str(&rest[..start]);
        let key = rest[start + 2..start + len].trim();
        match lookup(key, vars) {
            Some(Value::String(v)) => out.push_str(&v),
            Some(v) => out.push_str(&v.to_string()),
            None => {
                missing.push(key.to_string());
                out.push_str(&rest[start..start + len + 2]);
            }
        }
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    Value::String(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fills_params_sets_and_keeps_types() {
        let t = Template::parse(
            "params: { sender: Ada, badge: 1 }\nmessage:\n  notification: { title: '{{sender}} says hi', body: '{{ sender }}' }\n  apns: { payload: { aps: { badge: '{{badge}}' } } }\n",
        )
        .unwrap();
        let m = t.render(&[("badge".into(), "3".into())]).unwrap();
        assert_eq!(m["notification"]["title"], "Ada says hi");
        assert_eq!(m["notification"]["body"], "Ada");
        assert_eq!(m["apns"]["payload"]["aps"]["badge"], json!(3));
    }

    #[test]
    fn names_every_missing_value() {
        let t = Template::parse("message: { data: { a: '{{x}}', b: 'id {{y}}', c: '{{x}}' } }")
            .unwrap();
        let err = t.render(&[]).unwrap_err();
        assert!(err.starts_with("no value for x, y:"), "{err}");
    }

    #[test]
    fn built_in_values() {
        let t = Template::parse(
            "message: { data: { e: '{{env.PATH}}', id: '{{uuid}}', t: '{{unix}}' } }",
        )
        .unwrap();
        let m = t.render(&[]).unwrap();
        assert_eq!(m["data"]["e"], std::env::var("PATH").unwrap().as_str());
        assert_eq!(m["data"]["id"].as_str().unwrap().len(), 36);
        assert!(m["data"]["t"].as_u64().unwrap() > 1_700_000_000);
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(Template::parse("mesage: {}").is_err());
    }
}
