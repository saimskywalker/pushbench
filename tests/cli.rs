//! The binary end to end, against a local HTTP server standing in for Google and Apple.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::JoinHandle;

#[derive(Debug)]
struct Request {
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

/// Answers one request per canned response, in order, and returns what it received.
fn serve(responses: Vec<(u16, &'static str, String)>) -> (String, JoinHandle<Vec<Request>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, extra_header, body) in responses {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let mut headers = HashMap::new();
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    headers.insert(k.trim().to_lowercase(), v.trim().to_string());
                }
            }
            let len = headers
                .get("content-length")
                .and_then(|l| l.parse().ok())
                .unwrap_or(0);
            let mut buf = vec![0; len];
            reader.read_exact(&mut buf).unwrap();
            seen.push(Request {
                path,
                headers,
                body: String::from_utf8_lossy(&buf).into(),
            });
            let mut stream = stream;
            let extra = if extra_header.is_empty() {
                String::new()
            } else {
                format!("{extra_header}\r\n")
            };
            write!(
                stream,
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n{extra}\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        seen
    });
    (url, handle)
}

fn workdir(config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pushbench-cli-{}", uuid_like()));
    std::fs::create_dir_all(dir.join("push")).unwrap();
    std::fs::write(dir.join("pushbench.toml"), config).unwrap();
    std::fs::write(
        dir.join("push/chat.yaml"),
        "params: { name: Ada }\nmessage:\n  notification: { title: '{{name}}', body: Hi }\n  data: { type: chat }\n",
    )
    .unwrap();
    dir
}

fn uuid_like() -> String {
    format!(
        "{:x}{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn pushbench(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, Value, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pushbench"));
    cmd.current_dir(dir)
        .args(args)
        .env("PUSHBENCH_HISTORY", dir.join("history.jsonl"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let json = serde_json::from_str(&stdout).unwrap_or(Value::Null);
    (
        out.status.code().unwrap_or(-1),
        json,
        format!("{stdout}{}", String::from_utf8_lossy(&out.stderr)),
    )
}

#[test]
fn fcm_with_gcloud_credentials_signs_in_sends_and_explains_errors() {
    let dir = workdir(
        "[fcm]\nproject = 'demo-dev'\ncredentials = 'adc.json'\n[targets]\npixel = 'fcm:token-pixel'\n",
    );
    std::fs::write(
        dir.join("adc.json"),
        json!({ "type": "authorized_user", "client_id": "cid", "client_secret": "sec", "refresh_token": "ref" }).to_string(),
    )
    .unwrap();
    let (url, server) = serve(vec![
        (200, "", json!({ "access_token": "at-1", "expires_in": 3599 }).to_string()),
        (200, "", json!({ "name": "projects/demo-dev/messages/0:123" }).to_string()),
        (
            404,
            "",
            json!({ "error": { "code": 404, "message": "Requested entity was not found.", "status": "NOT_FOUND",
                "details": [{ "@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError", "errorCode": "UNREGISTERED" }] } })
            .to_string(),
        ),
    ]);
    let env = [
        ("PUSHBENCH_FCM_URL", url.as_str()),
        ("PUSHBENCH_GOOGLE_TOKEN_URL", url.as_str()),
    ];
    let (code, out, all) = pushbench(
        &dir,
        &[
            "send",
            "chat",
            "--to",
            "pixel",
            "--to",
            "fcm:a-much-longer-inline-token",
            "--set",
            "name=Bo",
            "--json",
        ],
        &env,
    );
    assert_eq!(code, 1, "{all}");
    let results = out["results"].as_array().unwrap();
    assert_eq!(results[0]["ok"], true);
    assert_eq!(results[0]["id"], "0:123");
    assert_eq!(results[1]["ok"], false);
    assert_eq!(results[1]["code"], "UNREGISTERED");
    assert_eq!(results[1]["target"], "fcm:a-mu…oken");
    assert_eq!(results[1]["explanation"]["retryable"], false);

    let seen = server.join().unwrap();
    assert!(
        seen[0].body.contains("grant_type=refresh_token")
            && seen[0].body.contains("refresh_token=ref")
    );
    assert_eq!(seen[1].path, "/v1/projects/demo-dev/messages:send");
    assert_eq!(seen[1].headers["authorization"], "Bearer at-1");
    assert_eq!(seen[1].headers["x-goog-user-project"], "demo-dev");
    let body: Value = serde_json::from_str(&seen[1].body).unwrap();
    assert_eq!(body["message"]["token"], "token-pixel");
    assert_eq!(body["message"]["notification"]["title"], "Bo");
    assert_eq!(body["validate_only"], false);

    let history = std::fs::read_to_string(dir.join("history.jsonl")).unwrap();
    assert!(
        history.contains("\"pixel\"") && !history.contains("a-much-longer-inline-token"),
        "{history}"
    );
}

fn openssl_p8(dir: &Path) -> Option<PathBuf> {
    let path = dir.join("AuthKey_TEST.p8");
    let ok = Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "EC",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-out",
        ])
        .arg(&path)
        .output()
        .is_ok_and(|o| o.status.success());
    ok.then_some(path)
}

#[test]
fn apns_signs_a_jwt_and_sends_the_rendered_push() {
    let dir = workdir(
        "bundle_id = 'com.example.app'\nfcm_compat = false\n[apns]\nkey = 'AuthKey_TEST.p8'\nkey_id = 'KEY123'\nteam_id = 'TEAM42'\n",
    );
    if openssl_p8(&dir).is_none() {
        eprintln!("skipped: openssl is not available to make a test key");
        return;
    }
    std::fs::write(
        dir.join("push/live.yaml"),
        "message:\n  apns:\n    payload:\n      aps: { event: update, content-state: { score: 3 } }\n",
    )
    .unwrap();
    let (url, server) = serve(vec![
        (
            200,
            "apns-id: 11111111-2222-3333-4444-555555555555",
            String::new(),
        ),
        (400, "", json!({ "reason": "BadDeviceToken" }).to_string()),
    ]);
    let env = [("PUSHBENCH_APNS_URL", url.as_str())];
    let (code, out, all) = pushbench(
        &dir,
        &[
            "send",
            "live",
            "--to",
            "apns:devtoken1",
            "--to",
            "apns:devtoken2",
            "--json",
        ],
        &env,
    );
    assert_eq!(code, 1, "{all}");
    assert_eq!(
        out["results"][0]["id"],
        "11111111-2222-3333-4444-555555555555"
    );
    assert_eq!(out["results"][1]["code"], "BadDeviceToken");

    let seen = server.join().unwrap();
    assert_eq!(seen[0].path, "/3/device/devtoken1");
    assert_eq!(
        seen[0].headers["apns-topic"],
        "com.example.app.push-type.liveactivity"
    );
    assert_eq!(seen[0].headers["apns-push-type"], "liveactivity");
    assert_eq!(seen[0].headers["apns-priority"], "10");
    let jwt = seen[0].headers["authorization"]
        .strip_prefix("bearer ")
        .unwrap();
    let header: Value = serde_json::from_slice(&base64url(jwt.split('.').next().unwrap())).unwrap();
    assert_eq!(header["alg"], "ES256");
    assert_eq!(header["kid"], "KEY123");
    let claims: Value = serde_json::from_slice(&base64url(jwt.split('.').nth(1).unwrap())).unwrap();
    assert_eq!(claims["iss"], "TEAM42");
    let payload: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(payload["aps"]["content-state"]["score"], 3);
    assert!(payload["aps"]["timestamp"].as_u64().is_some());
    assert!(payload.get("gcm.message_id").is_none());
}

fn base64url(s: &str) -> Vec<u8> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        bits = (bits << 6) | alphabet.iter().position(|&a| a == c).unwrap() as u32;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    out
}

#[test]
fn dry_run_shows_the_simulator_payload_without_sending() {
    let dir = workdir("bundle_id = 'com.example.app'\n");
    let (code, out, all) = pushbench(
        &dir,
        &["send", "chat", "--to", "sim", "--dry-run", "--json"],
        &[],
    );
    assert_eq!(code, 0, "{all}");
    let request = &out["results"][0]["request"];
    assert_eq!(
        request["command"],
        json!(["xcrun", "simctl", "push", "booted", "com.example.app", "-"])
    );
    assert_eq!(request["payload"]["aps"]["alert"]["title"], "Ada");
    assert_eq!(request["payload"]["type"], "chat");
    assert!(request["payload"]["gcm.message_id"].is_string());
    assert!(!dir.join("history.jsonl").exists());
}

#[test]
fn lint_errors_stop_a_send_and_fail_lint() {
    let dir = workdir("bundle_id = 'com.example.app'\n");
    std::fs::write(
        dir.join("push/bad.yaml"),
        "message:\n  data: { count: 3 }\n",
    )
    .unwrap();
    let (code, out, _) = pushbench(&dir, &["send", "bad", "--to", "sim", "--json"], &[]);
    assert_eq!(code, 2);
    assert_eq!(out["lint"][0]["at"], "data.count");
    let (code, _, all) = pushbench(&dir, &["lint"], &[]);
    assert_eq!(code, 2);
    assert!(all.contains("✓") && all.contains("data.count"), "{all}");
}

#[test]
fn init_creates_a_working_setup_and_ignores_the_local_file() {
    let dir = std::env::temp_dir().join(format!("pushbench-init-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    let (code, _, all) = pushbench(&dir, &["init"], &[]);
    assert_eq!(code, 0, "{all}");
    assert!(
        std::fs::read_to_string(dir.join(".gitignore"))
            .unwrap()
            .contains("pushbench.local.toml")
    );
    let (code, _, all) = pushbench(&dir, &["lint"], &[]);
    assert_eq!(code, 0, "{all}");
    let (code, out, all) = pushbench(
        &dir,
        &[
            "render", "hello", "--for", "fcm", "--set", "name=Cy", "--json",
        ],
        &[],
    );
    assert_eq!(code, 0, "{all}");
    assert_eq!(
        out["message"]["notification"]["title"],
        "Cy sent you a message"
    );
    let (code, _, _) = pushbench(&dir, &["init"], &[]);
    assert_eq!(code, 2);
}

#[test]
fn explain_and_unknown_targets() {
    let dir = workdir("");
    let (code, out, _) = pushbench(&dir, &["explain", "SENDER_ID_MISMATCH", "--json"], &[]);
    assert_eq!(code, 0);
    assert_eq!(out[0]["code"], "SENDER_ID_MISMATCH");
    let (code, out, _) = pushbench(&dir, &["send", "chat", "--to", "pager", "--json"], &[]);
    assert_eq!(code, 2);
    assert!(out["error"].as_str().unwrap().starts_with("unknown target"));
}
