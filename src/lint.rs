//! Checks a rendered message against what FCM, APNs and the devices accept.

use crate::render;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
}

#[derive(Debug, Serialize)]
pub struct Finding {
    pub level: Level,
    /// Where in the message, e.g. `data.count`.
    pub at: String,
    pub message: String,
}

const MESSAGE_KEYS: &[&str] = &[
    "notification",
    "data",
    "android",
    "apns",
    "webpush",
    "fcm_options",
];
const NOTIFICATION_KEYS: &[&str] = &["title", "body", "image"];
const PUSH_TYPES: &[&str] = &[
    "alert",
    "background",
    "voip",
    "liveactivity",
    "complication",
    "fileprovider",
    "location",
    "widgets",
    "mdm",
];
const MAX_FCM: usize = 4096;
const MAX_APNS: usize = 4096;
const MAX_APNS_VOIP: usize = 5120;

pub fn lint(message: &Value) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut add = |level, at: &str, message: String| {
        out.push(Finding {
            level,
            at: at.into(),
            message,
        })
    };
    use Level::{Error, Warning};

    let Some(top) = message.as_object() else {
        add(Error, "message", "must be a mapping".into());
        return out;
    };
    for key in top.keys() {
        if ["token", "topic", "condition"].contains(&key.as_str()) {
            add(
                Error,
                key,
                "the destination comes from --to, not the template".into(),
            );
        } else if !MESSAGE_KEYS.contains(&key.as_str()) {
            add(
                Error,
                key,
                format!(
                    "not an FCM message field (expected one of {})",
                    MESSAGE_KEYS.join(", ")
                ),
            );
        }
    }
    if let Some(n) = top.get("notification").and_then(Value::as_object) {
        for key in n
            .keys()
            .filter(|k| !NOTIFICATION_KEYS.contains(&k.as_str()))
        {
            add(
                Error,
                &format!("notification.{key}"),
                "FCM allows only title, body and image here; put the rest under android or apns"
                    .into(),
            );
        }
    }
    if let Some(data) = top.get("data").and_then(Value::as_object) {
        for (key, value) in data {
            let at = format!("data.{key}");
            if !value.is_string() {
                add(
                    Error,
                    &at,
                    format!(
                        "FCM data values must be strings: write \"{}\"",
                        value
                            .as_str()
                            .map_or_else(|| value.to_string(), str::to_string)
                    ),
                );
            }
            if ["from", "notification", "message_type"].contains(&key.as_str())
                || key.starts_with("google.")
                || key.starts_with("gcm.")
            {
                add(Error, &at, "reserved key: FCM rejects it".into());
            }
        }
    }
    let fcm_size = serde_json::to_vec(&serde_json::json!([
        top.get("notification"),
        top.get("data")
    ]))
    .map_or(0, |v| v.len());
    if fcm_size > MAX_FCM {
        add(
            Error,
            "message",
            format!("notification and data are {fcm_size} bytes; FCM allows {MAX_FCM}"),
        );
    }

    let android = &message["android"];
    if let Some(p) = android.get("priority").and_then(Value::as_str)
        && !["normal", "high"].contains(&p.to_lowercase().as_str())
    {
        add(Error, "android.priority", "must be normal or high".into());
    }
    if let Some(ttl) = android.get("ttl")
        && !ttl
            .as_str()
            .is_some_and(|t| t.ends_with('s') && t[..t.len() - 1].parse::<f64>().is_ok())
    {
        add(
            Error,
            "android.ttl",
            "must be a duration string such as \"3600s\"".into(),
        );
    }

    let headers = &message["apns"]["headers"];
    if let Some(p) = headers.get("apns-priority")
        && !["1", "5", "10"].contains(
            &p.as_str()
                .map_or_else(|| p.to_string(), str::to_string)
                .as_str(),
        )
    {
        add(
            Error,
            "apns.headers.apns-priority",
            "must be 1, 5 or 10".into(),
        );
    }
    if let Some(t) = headers.get("apns-push-type").and_then(Value::as_str)
        && !PUSH_TYPES.contains(&t)
    {
        add(
            Error,
            "apns.headers.apns-push-type",
            format!("must be one of {}", PUSH_TYPES.join(", ")),
        );
    }

    let apns = render::apns(message, "lint", true);
    let aps = &apns.payload["aps"];
    let limit = if apns.push_type == "voip" {
        MAX_APNS_VOIP
    } else {
        MAX_APNS
    };
    let size = serde_json::to_vec(&apns.payload).map_or(0, |v| v.len());
    if size > limit {
        add(
            Error,
            "apns.payload",
            format!("the iOS payload is {size} bytes; APNs allows {limit}"),
        );
    }
    if apns.push_type == "background" {
        if aps.get("content-available") != Some(&Value::from(1)) {
            add(
                Warning,
                "apns.payload.aps",
                "iOS ignores a background push without `content-available: 1`".into(),
            );
        }
        if headers
            .get("apns-priority")
            .is_some_and(|p| p == "10" || p == 10)
        {
            add(
                Error,
                "apns.headers.apns-priority",
                "a background push must use priority 5".into(),
            );
        }
    }
    if apns.push_type == "liveactivity" {
        match aps.get("event").and_then(Value::as_str) {
            Some("start" | "update" | "end") => {}
            _ => add(
                Error,
                "apns.payload.aps.event",
                "must be start, update or end".into(),
            ),
        }
        let event = aps.get("event").and_then(Value::as_str);
        if matches!(event, Some("start" | "update")) && aps.get("content-state").is_none() {
            add(
                Error,
                "apns.payload.aps.content-state",
                "a Live Activity start or update needs content-state".into(),
            );
        }
        if event == Some("start") {
            for key in ["attributes-type", "attributes"] {
                if aps.get(key).is_none() {
                    add(
                        Error,
                        &format!("apns.payload.aps.{key}"),
                        "a Live Activity start needs it".into(),
                    );
                }
            }
        }
    }
    if apns.payload.get("fcm_options").is_some() {
        add(
            Warning,
            "notification.image",
            "iOS shows the image only if the app has a Notification Service Extension".into(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn codes(m: Value) -> Vec<(Level, String)> {
        lint(&m).into_iter().map(|f| (f.level, f.at)).collect()
    }

    #[test]
    fn a_plain_alert_is_clean() {
        assert!(lint(&json!({ "notification": { "title": "Hi", "body": "There" }, "data": { "type": "chat" } })).is_empty());
    }

    #[test]
    fn fcm_rules() {
        let found = codes(json!({
            "token": "x",
            "notifcation": {},
            "notification": { "title": "a", "sound": "default" },
            "data": { "count": 3, "google.x": "y" },
            "android": { "priority": "max", "ttl": 60 }
        }));
        for at in [
            "token",
            "notifcation",
            "notification.sound",
            "data.count",
            "data.google.x",
            "android.priority",
            "android.ttl",
        ] {
            assert!(
                found.iter().any(|(l, a)| *l == Level::Error && a == at),
                "missing {at} in {found:?}"
            );
        }
    }

    #[test]
    fn size_limits() {
        let big = "x".repeat(5000);
        let found = codes(json!({ "data": { "blob": big } }));
        assert!(found.contains(&(Level::Error, "message".into())));
        assert!(found.contains(&(Level::Error, "apns.payload".into())));
    }

    #[test]
    fn background_and_live_activity_rules() {
        let found = codes(
            json!({ "data": { "sync": "1" }, "apns": { "headers": { "apns-priority": "10" } } }),
        );
        assert!(found.contains(&(Level::Warning, "apns.payload.aps".into())));
        assert!(found.contains(&(Level::Error, "apns.headers.apns-priority".into())));

        let found = codes(json!({ "apns": { "payload": { "aps": { "event": "start" } } } }));
        for at in [
            "apns.payload.aps.content-state",
            "apns.payload.aps.attributes-type",
            "apns.payload.aps.attributes",
        ] {
            assert!(found.contains(&(Level::Error, at.into())), "missing {at}");
        }
    }

    #[test]
    fn image_warns_about_the_extension() {
        let found = codes(
            json!({ "notification": { "title": "a", "image": "https://example.com/a.png" } }),
        );
        assert_eq!(found, vec![(Level::Warning, "notification.image".into())]);
    }
}
