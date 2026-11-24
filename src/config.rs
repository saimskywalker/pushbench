//! `pushbench.toml` (committed, no secrets) and `pushbench.local.toml` (git-ignored:
//! tokens and key paths), and the targets they name.

use crate::render::FcmTo;
use crate::template::fill_all;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FILE: &str = "pushbench.toml";
pub const LOCAL_FILE: &str = "pushbench.local.toml";

pub struct Config {
    /// The folder holding `pushbench.toml`, or the working folder if there is none.
    pub dir: PathBuf,
    pub found: bool,
    root: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Sim,
    Fcm(FcmTo),
    Apns,
}

/// A resolved target: where to send, plus the settings that apply to it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    #[serde(skip_deserializing, default)]
    pub name: String,
    #[serde(skip_deserializing, default = "no_kind")]
    pub kind: Kind,
    /// The device for `sim`, the token for `fcm`/`apns`.
    #[serde(skip_deserializing)]
    pub address: String,
    pub bundle_id: Option<String>,
    /// Android application id, for `--state` on an adb device.
    pub package: Option<String>,
    #[serde(default = "yes")]
    pub fcm_compat: bool,
    pub project: Option<String>,
    pub credentials: Option<String>,
    pub key: Option<String>,
    pub key_id: Option<String>,
    pub team_id: Option<String>,
    pub environment: Option<String>,
    /// adb serial of the device behind an FCM target, for `--state`.
    pub adb: Option<String>,
}

fn no_kind() -> Kind {
    Kind::Sim
}

fn yes() -> bool {
    true
}

impl Config {
    /// Finds `pushbench.toml` in `start` or a parent folder.
    pub fn load(start: &Path) -> Result<Self, String> {
        let Some(dir) = start.ancestors().find(|d| d.join(FILE).is_file()) else {
            return Ok(Self {
                dir: start.to_path_buf(),
                found: false,
                root: Map::new(),
            });
        };
        let mut root = read(&dir.join(FILE))?;
        if dir.join(LOCAL_FILE).is_file() {
            merge(&mut root, read(&dir.join(LOCAL_FILE))?);
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            found: true,
            root,
        })
    }

    /// Whether the config has a top-level key or section, e.g. `fcm`.
    pub fn has(&self, key: &str) -> bool {
        self.root.contains_key(key)
    }

    pub fn templates_dir(&self) -> PathBuf {
        self.path(
            self.root
                .get("templates")
                .and_then(Value::as_str)
                .unwrap_or("push"),
        )
    }

    /// `~/x` from home, a relative path from the config folder.
    pub fn path(&self, p: &str) -> PathBuf {
        match p.strip_prefix("~/") {
            Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
            None => self.dir.join(p),
        }
    }

    /// Target names and what they point at, as written.
    pub fn aliases(&self) -> BTreeMap<String, String> {
        let Some(Value::Object(targets)) = self.root.get("targets") else {
            return BTreeMap::new();
        };
        targets
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_str().or(v["to"].as_str()).unwrap_or("?").to_string(),
                )
            })
            .collect()
    }

    /// A target from an alias in `[targets]` or a spec such as `fcm:<token>`.
    pub fn target(&self, name: &str) -> Result<Target, String> {
        let entry = self.root.get("targets").and_then(|t| t.get(name)).cloned();
        let (spec, overrides) = match entry {
            Some(Value::String(spec)) => (spec, Map::new()),
            Some(Value::Object(mut o)) => match o.remove("to") {
                Some(Value::String(spec)) => (spec, o),
                _ => return Err(format!("target `{name}` needs `to = \"...\"`")),
            },
            Some(_) => return Err(format!("target `{name}` must be a string or a table")),
            None => (name.to_string(), Map::new()),
        };
        let (kind, section, address) = parse_spec(&spec)?;

        let mut settings = Map::new();
        for key in ["bundle_id", "package", "fcm_compat"] {
            if let Some(v) = self.root.get(key) {
                settings.insert(key.into(), v.clone());
            }
        }
        if let Some(Value::Object(s)) = self.root.get(section) {
            merge(&mut settings, s.clone());
        }
        merge(&mut settings, overrides);
        let settings = fill_all(&Value::Object(settings), &BTreeMap::new())
            .map_err(|e| format!("target `{name}`: {e}"))?;
        let address = fill_all(&Value::String(address), &BTreeMap::new())
            .map_err(|e| format!("target `{name}`: {e}"))?;

        let mut target: Target = serde_json::from_value(settings)
            .map_err(|e| format!("target `{name}` ([{section}] or [targets]): {e}"))?;
        target.address = address.as_str().unwrap_or_default().to_string();
        for path in [&mut target.key, &mut target.credentials]
            .into_iter()
            .flatten()
        {
            *path = self.path(path).display().to_string();
        }
        target.kind = match kind {
            Kind::Fcm(to) => Kind::Fcm(match to {
                FcmTo::Token(_) => FcmTo::Token(target.address.clone()),
                FcmTo::Topic(_) => FcmTo::Topic(target.address.clone()),
                FcmTo::Condition(_) => FcmTo::Condition(target.address.clone()),
            }),
            k => k,
        };
        target.name = name.to_string();
        Ok(target)
    }
}

/// `sim`, `sim:<device>`, `fcm:<token>`, `topic:<name>`, `condition:<expr>`, `apns:<token>`.
fn parse_spec(spec: &str) -> Result<(Kind, &'static str, String), String> {
    let (scheme, rest) = spec.split_once(':').unwrap_or((spec, ""));
    let rest = rest.to_string();
    let need = |what: &str| {
        if rest.is_empty() {
            Err(format!("`{spec}`: give the {what} after `{scheme}:`"))
        } else {
            Ok(())
        }
    };
    Ok(match scheme {
        "sim" => (
            Kind::Sim,
            "sim",
            if rest.is_empty() {
                "booted".into()
            } else {
                rest
            },
        ),
        "fcm" => {
            need("device token").map(|_| (Kind::Fcm(FcmTo::Token(String::new())), "fcm", rest))?
        }
        "topic" => need("topic").map(|_| (Kind::Fcm(FcmTo::Topic(String::new())), "fcm", rest))?,
        "condition" => {
            need("condition").map(|_| (Kind::Fcm(FcmTo::Condition(String::new())), "fcm", rest))?
        }
        "apns" => need("device token").map(|_| (Kind::Apns, "apns", rest))?,
        _ => {
            return Err(format!(
                "unknown target `{spec}`: use a name from [targets], or sim, sim:<device>, fcm:<token>, topic:<name>, condition:<expr>, apns:<token>"
            ));
        }
    })
}

fn read(path: &Path) -> Result<Map<String, Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn merge(into: &mut Map<String, Value>, from: Map<String, Value>) {
    for (k, v) in from {
        match (into.get_mut(&k), v) {
            (Some(Value::Object(a)), Value::Object(b)) => merge(a, b),
            (_, v) => {
                into.insert(k, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(toml_text: &str, local: Option<&str>) -> Config {
        let dir = std::env::temp_dir().join(format!("pushbench-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("app/lib")).unwrap();
        std::fs::write(dir.join("app").join(FILE), toml_text).unwrap();
        if let Some(l) = local {
            std::fs::write(dir.join("app").join(LOCAL_FILE), l).unwrap();
        }
        Config::load(&dir.join("app/lib")).unwrap()
    }

    #[test]
    fn aliases_merge_sections_and_the_local_file() {
        let c = config(
            "bundle_id = 'com.example.app'\n[apns]\nkey_id = 'K'\nteam_id = 'T'\nenvironment = 'sandbox'\n[targets]\nphone = { to = 'apns:tok123', environment = 'production' }\nsim = 'sim'\n",
            Some("[apns]\nkey = '~/keys/AuthKey_K.p8'\n"),
        );
        assert!(c.found);
        let t = c.target("phone").unwrap();
        assert_eq!(t.kind, Kind::Apns);
        assert_eq!(t.address, "tok123");
        assert_eq!(t.environment.as_deref(), Some("production"));
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            t.key,
            Some(home.join("keys/AuthKey_K.p8").display().to_string())
        );
        assert_eq!(t.bundle_id.as_deref(), Some("com.example.app"));
        assert!(t.fcm_compat);
        assert_eq!(c.target("sim").unwrap().address, "booted");
        assert_eq!(c.aliases()["phone"], "apns:tok123");
    }

    #[test]
    fn inline_specs() {
        let c = config("[fcm]\nproject = 'demo'\n", None);
        let t = c.target("topic:news").unwrap();
        assert_eq!(t.kind, Kind::Fcm(FcmTo::Topic("news".into())));
        assert_eq!(t.project.as_deref(), Some("demo"));
        assert_eq!(c.target("sim:iPhone 17").unwrap().address, "iPhone 17");
        assert!(c.target("fcm:").unwrap_err().contains("device token"));
        assert!(c.target("pager").unwrap_err().starts_with("unknown target"));
    }

    #[test]
    fn typos_in_settings_are_errors() {
        let c = config("[targets]\nx = { to = 'sim', bundel_id = 'a' }\n", None);
        assert!(c.target("x").unwrap_err().contains("bundel_id"));
    }
}
