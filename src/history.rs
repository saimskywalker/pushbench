//! A local log of sends for `pushbench history`. It keeps target names, never tokens or payloads.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub time: u64,
    pub template: Option<String>,
    #[serde(default)]
    pub sets: Vec<(String, String)>,
    pub title: Option<String>,
    pub body: Option<String>,
    /// Target names; inline tokens are stored masked, so those can't be resent.
    pub to: Vec<String>,
    pub ok: bool,
}

fn path() -> PathBuf {
    std::env::var_os("PUSHBENCH_HISTORY")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_default()
                .join("pushbench/history.jsonl")
        })
}

/// `fcm:abcd…wxyz`: enough to recognise a token, not enough to use it.
pub fn mask(spec: &str) -> String {
    match spec.split_once(':') {
        Some((scheme, rest)) if matches!(scheme, "fcm" | "apns") && rest.chars().count() > 12 => {
            let chars: Vec<char> = rest.chars().collect();
            let head: String = chars[..4].iter().collect();
            let tail: String = chars[chars.len() - 4..].iter().collect();
            format!("{scheme}:{head}…{tail}")
        }
        _ => spec.to_string(),
    }
}

pub fn append(entry: &Entry) -> Result<(), String> {
    append_to(&path(), entry)
}

/// Every entry, oldest first; unreadable lines are skipped.
pub fn read() -> Vec<Entry> {
    read_from(&path())
}

fn append_to(path: &std::path::Path, entry: &Entry) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    writeln!(file, "{line}").map_err(|e| format!("{}: {e}", path.display()))
}

fn read_from(path: &std::path::Path) -> Vec<Entry> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_inline_tokens_only() {
        assert_eq!(mask("fcm:dGhpcyBpcyBhIHRva2Vu"), "fcm:dGhp…a2Vu");
        assert_eq!(mask("apns:short"), "apns:short");
        assert_eq!(mask("pixel"), "pixel");
        assert_eq!(mask("topic:news"), "topic:news");
    }

    #[test]
    fn appends_and_reads_back() {
        let file =
            std::env::temp_dir().join(format!("pushbench-history-{}.jsonl", uuid::Uuid::new_v4()));
        let e = Entry {
            time: 1,
            template: Some("chat".into()),
            sets: vec![("a".into(), "b".into())],
            title: None,
            body: None,
            to: vec!["sim".into()],
            ok: true,
        };
        append_to(&file, &e).unwrap();
        append_to(
            &file,
            &Entry {
                ok: false,
                ..e.clone()
            },
        )
        .unwrap();
        assert_eq!(read_from(&file), vec![e.clone(), Entry { ok: false, ..e }]);
        std::fs::remove_file(file).unwrap();
    }
}
