//! One FCM v1 message, rendered for each way of delivering it.

use serde_json::{Map, Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

/// APNs topic suffix for push types that need one; others use the bundle id as is.
const TOPIC_SUFFIX: &[(&str, &str)] = &[
    ("voip", ".voip"),
    ("liveactivity", ".push-type.liveactivity"),
    ("complication", ".complication"),
    ("fileprovider", ".pushkit.fileprovider"),
    ("location", ".location-query"),
    ("widgets", ".push-type.widgets"),
];

#[derive(Debug)]
pub struct ApnsPush {
    pub push_type: String,
    pub headers: Vec<(String, String)>,
    pub payload: Value,
}

/// The APNs request for `message`: the payload a device (or `simctl push`) gets, and its headers.
///
/// Mirrors what FCM itself sends to iOS: `notification` becomes `aps.alert`, `data` keys go
/// to the top level, and an image sets `mutable-content`. With `fcm_compat` it adds the
/// `gcm.message_id` that the Firebase iOS SDK needs before it hands a push to the app.
pub fn apns(message: &Value, bundle_id: &str, fcm_compat: bool) -> ApnsPush {
    let apns = &message["apns"];
    let notification = &message["notification"];
    let mut payload = apns["payload"].as_object().cloned().unwrap_or_default();
    let mut aps = match payload.remove("aps") {
        Some(Value::Object(aps)) => aps,
        _ => Map::new(),
    };

    if !aps.contains_key("alert") {
        let alert: Map<String, Value> = ["title", "body"]
            .into_iter()
            .filter_map(|k| Some((k.to_string(), notification.get(k)?.clone())))
            .collect();
        if !alert.is_empty() {
            aps.insert("alert".into(), alert.into());
        }
    }
    if let Some(image) = apns["fcm_options"]["image"]
        .as_str()
        .or(notification["image"].as_str())
    {
        aps.entry("mutable-content").or_insert(1.into());
        payload.insert("fcm_options".into(), json!({ "image": image }));
    }
    if let Some(data) = message["data"].as_object() {
        for (k, v) in data {
            payload.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    let given: Map<String, Value> = apns["headers"].as_object().cloned().unwrap_or_default();
    let header = |name: &str| given.get(name).map(text);
    let push_type = header("apns-push-type").unwrap_or_else(|| {
        if aps.contains_key("event") {
            "liveactivity".into()
        } else if ["alert", "badge", "sound"]
            .iter()
            .any(|k| aps.contains_key(*k))
        {
            "alert".into()
        } else {
            "background".into()
        }
    });
    if push_type == "liveactivity" {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        aps.entry("timestamp").or_insert(now.into());
    }
    if fcm_compat {
        payload
            .entry("gcm.message_id")
            .or_insert_with(|| uuid::Uuid::new_v4().to_string().into());
        payload.entry("google.c.a.e").or_insert("1".into());
    }
    payload.insert("aps".into(), aps.into());

    let suffix = TOPIC_SUFFIX
        .iter()
        .find(|(t, _)| *t == push_type)
        .map_or("", |(_, s)| s);
    let mut headers = vec![
        ("apns-push-type".to_string(), push_type.clone()),
        (
            "apns-topic".to_string(),
            header("apns-topic").unwrap_or(format!("{bundle_id}{suffix}")),
        ),
        (
            "apns-priority".to_string(),
            header("apns-priority")
                .unwrap_or_else(|| if push_type == "background" { "5" } else { "10" }.into()),
        ),
    ];
    for (k, v) in &given {
        if !headers.iter().any(|(h, _)| h == k) {
            headers.push((k.clone(), text(v)));
        }
    }
    ApnsPush {
        push_type,
        headers,
        payload: payload.into(),
    }
}

/// Where an FCM message goes: one of `token`, `topic` or `condition`.
#[derive(Debug, Clone, PartialEq)]
pub enum FcmTo {
    Token(String),
    Topic(String),
    Condition(String),
}

/// The body for `projects/{id}/messages:send`.
pub fn fcm(message: &Value, to: &FcmTo, validate_only: bool) -> Value {
    let mut message = message.as_object().cloned().unwrap_or_default();
    let (key, value) = match to {
        FcmTo::Token(t) => ("token", t),
        FcmTo::Topic(t) => ("topic", t),
        FcmTo::Condition(c) => ("condition", c),
    };
    message.insert(key.into(), value.clone().into());
    json!({ "message": message, "validate_only": validate_only })
}

fn text(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header<'a>(p: &'a ApnsPush, name: &str) -> &'a str {
        &p.headers.iter().find(|(k, _)| k == name).unwrap().1
    }

    #[test]
    fn alert_with_data_and_image_matches_what_fcm_sends() {
        let m = json!({
            "notification": { "title": "Ada", "body": "Hi", "image": "https://example.com/a.png" },
            "data": { "type": "chat", "thread_id": "t1" },
            "apns": { "payload": { "aps": { "badge": 3 } } }
        });
        let p = apns(&m, "com.example.app", true);
        assert_eq!(p.push_type, "alert");
        assert_eq!(
            p.payload["aps"]["alert"],
            json!({ "title": "Ada", "body": "Hi" })
        );
        assert_eq!(p.payload["aps"]["badge"], 3);
        assert_eq!(p.payload["aps"]["mutable-content"], 1);
        assert_eq!(
            p.payload["fcm_options"]["image"],
            "https://example.com/a.png"
        );
        assert_eq!(p.payload["thread_id"], "t1");
        assert_eq!(p.payload["google.c.a.e"], "1");
        assert!(p.payload["gcm.message_id"].is_string());
        assert_eq!(header(&p, "apns-topic"), "com.example.app");
        assert_eq!(header(&p, "apns-priority"), "10");
    }

    #[test]
    fn data_only_is_a_background_push_at_priority_5() {
        let m = json!({ "data": { "sync": "1" }, "apns": { "payload": { "aps": { "content-available": 1 } } } });
        let p = apns(&m, "com.example.app", false);
        assert_eq!(p.push_type, "background");
        assert_eq!(header(&p, "apns-priority"), "5");
        assert!(p.payload.get("gcm.message_id").is_none());
    }

    #[test]
    fn push_types_get_their_topic_suffix_and_live_activities_a_timestamp() {
        let m = json!({ "apns": { "payload": { "aps": { "event": "update", "content-state": { "score": 2 } } } } });
        let p = apns(&m, "com.example.app", false);
        assert_eq!(p.push_type, "liveactivity");
        assert_eq!(
            header(&p, "apns-topic"),
            "com.example.app.push-type.liveactivity"
        );
        assert!(p.payload["aps"]["timestamp"].as_u64().unwrap() > 1_700_000_000);

        let voip = json!({ "apns": { "headers": { "apns-push-type": "voip", "apns-expiration": 0 }, "payload": { "aps": {} } } });
        let p = apns(&voip, "com.example.app", false);
        assert_eq!(header(&p, "apns-topic"), "com.example.app.voip");
        assert_eq!(header(&p, "apns-expiration"), "0");
    }

    #[test]
    fn given_headers_and_alert_win() {
        let m = json!({
            "notification": { "title": "ignored" },
            "apns": { "headers": { "apns-topic": "custom", "apns-priority": "5" }, "payload": { "aps": { "alert": "plain" } } }
        });
        let p = apns(&m, "com.example.app", false);
        assert_eq!(p.payload["aps"]["alert"], "plain");
        assert_eq!(header(&p, "apns-topic"), "custom");
        assert_eq!(header(&p, "apns-priority"), "5");
    }

    #[test]
    fn fcm_body_adds_the_target() {
        let m = json!({ "notification": { "title": "Hi" } });
        let b = fcm(&m, &FcmTo::Topic("news".into()), true);
        assert_eq!(
            b,
            json!({ "message": { "notification": { "title": "Hi" }, "topic": "news" }, "validate_only": true })
        );
    }
}
