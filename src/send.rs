//! Delivery through `xcrun simctl push`, FCM HTTP v1 and APNs.

use crate::config::{Kind, Target};
use crate::device;
use crate::explain::{Explanation, explain};
use crate::render::{self, FcmTo};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::blocking::Client;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub target: String,
    pub via: &'static str,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<&'static Explanation>,
    /// With --dry-run: what would have been sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Value>,
}

impl Outcome {
    fn new(target: &Target, via: &'static str) -> Self {
        Self {
            target: target.name.clone(),
            via,
            ok: true,
            id: None,
            status: None,
            code: None,
            detail: None,
            explanation: None,
            request: None,
        }
    }

    fn rejected(mut self, status: Option<u16>, code: String, detail: Option<String>) -> Self {
        self.ok = false;
        self.status = status;
        self.explanation = explain(&code);
        self.code = Some(code);
        self.detail = detail.filter(|d| !d.is_empty());
        self
    }
}

#[derive(Clone)]
struct GoogleAuth {
    token: String,
    project: Option<String>,
    /// User credentials need the project named in `x-goog-user-project`.
    user: bool,
}

pub struct Sender {
    http: Client,
    google: HashMap<PathBuf, GoogleAuth>,
    pub dry_run: bool,
    pub validate_only: bool,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn need<'a>(value: &'a Option<String>, what: &str, target: &Target) -> Result<&'a str, String> {
    value.as_deref().filter(|v| !v.is_empty()).ok_or_else(|| {
        format!(
            "target `{}` needs `{what}` (in pushbench.toml or pushbench.local.toml)",
            target.name
        )
    })
}

/// Where `gcloud auth application-default login` writes its credentials.
pub fn adc_path() -> PathBuf {
    if let Some(p) = std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS") {
        return p.into();
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        dirs::home_dir().map(|h| h.join(".config"))
    };
    base.unwrap_or_default()
        .join("gcloud/application_default_credentials.json")
}

impl Sender {
    pub fn new(dry_run: bool, validate_only: bool) -> Result<Self, String> {
        let http = Client::builder()
            .user_agent(concat!("pushbench/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("HTTP client: {e}"))?;
        Ok(Self {
            http,
            google: HashMap::new(),
            dry_run,
            validate_only,
        })
    }

    pub fn send(&mut self, target: &Target, message: &Value) -> Result<Outcome, String> {
        match &target.kind {
            Kind::Sim => self.sim(target, message),
            Kind::Fcm(to) => self.fcm(target, to, message),
            Kind::Apns => self.apns(target, message),
        }
    }

    fn sim(&self, target: &Target, message: &Value) -> Result<Outcome, String> {
        let bundle = need(&target.bundle_id, "bundle_id", target)?;
        let push = render::apns(message, bundle, target.fcm_compat);
        let mut out = Outcome::new(target, "sim");
        if self.dry_run {
            out.request = Some(
                json!({ "command": ["xcrun", "simctl", "push", target.address, bundle, "-"], "payload": push.payload }),
            );
            return Ok(out);
        }
        let payload = serde_json::to_vec(&push.payload).map_err(|e| e.to_string())?;
        match device::push_to_sim(&target.address, bundle, &payload) {
            Ok(_) => Ok(out),
            Err(e) if e.starts_with("could not run") => {
                Err(format!("{e} (the iOS Simulator needs macOS with Xcode)"))
            }
            Err(e) => {
                let code = if e.contains("No devices are booted") || e.contains("Invalid device") {
                    "SimulatorNotBooted"
                } else if e.contains("not authorized") {
                    "SimulatorNotAuthorized"
                } else {
                    "SimulatorPushFailed"
                };
                Ok(out.rejected(None, code.into(), Some(e)))
            }
        }
    }

    fn fcm(&mut self, target: &Target, to: &FcmTo, message: &Value) -> Result<Outcome, String> {
        let body = render::fcm(message, to, self.validate_only);
        let base = env_or("PUSHBENCH_FCM_URL", "https://fcm.googleapis.com");
        let mut out = Outcome::new(target, "fcm");
        if self.dry_run {
            let project = target.project.as_deref().unwrap_or("<project>");
            out.request = Some(
                json!({ "url": format!("{base}/v1/projects/{project}/messages:send"), "body": body }),
            );
            return Ok(out);
        }
        let auth = self.google_auth(target)?;
        let project = target.project.clone().or(auth.project.clone()).ok_or_else(|| {
            format!("target `{}` needs `project` under [fcm]: the Firebase project id the app is registered in", target.name)
        })?;
        let mut req = self
            .http
            .post(format!("{base}/v1/projects/{project}/messages:send"))
            .bearer_auth(&auth.token)
            .json(&body);
        if auth.user {
            req = req.header("x-goog-user-project", &project);
        }
        let resp = req.send().map_err(|e| format!("FCM: {e}"))?;
        let status = resp.status().as_u16();
        let v: Value = resp.json().unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            out.status = Some(status);
            out.id = v["name"]
                .as_str()
                .map(|n| n.rsplit('/').next().unwrap_or(n).to_string());
            return Ok(out);
        }
        let err = &v["error"];
        let code = err["details"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|d| d["errorCode"].as_str())
            .or(err["status"].as_str())
            .unwrap_or("HTTP_ERROR")
            .to_string();
        Ok(out.rejected(
            Some(status),
            code,
            err["message"].as_str().map(str::to_string),
        ))
    }

    fn google_auth(&mut self, target: &Target) -> Result<GoogleAuth, String> {
        let path = target
            .credentials
            .as_ref()
            .map_or_else(adc_path, PathBuf::from);
        if let Some(auth) = self.google.get(&path) {
            return Ok(auth.clone());
        }
        let auth = google_token(&self.http, &path)?;
        self.google.insert(path, auth.clone());
        Ok(auth)
    }

    fn apns(&self, target: &Target, message: &Value) -> Result<Outcome, String> {
        let bundle = need(&target.bundle_id, "bundle_id", target)?;
        let push = render::apns(message, bundle, target.fcm_compat);
        let host = match target.environment.as_deref().unwrap_or("sandbox") {
            "sandbox" | "development" => "https://api.sandbox.push.apple.com",
            "production" => "https://api.push.apple.com",
            other => {
                return Err(format!(
                    "target `{}`: environment must be sandbox or production, not `{other}`",
                    target.name
                ));
            }
        };
        let url = format!(
            "{}/3/device/{}",
            env_or("PUSHBENCH_APNS_URL", host),
            target.address
        );
        let mut out = Outcome::new(target, "apns");
        if self.dry_run {
            let headers: serde_json::Map<String, Value> = push
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().into()))
                .collect();
            out.request = Some(json!({ "url": url, "headers": headers, "payload": push.payload }));
            return Ok(out);
        }
        let jwt = apns_jwt(
            need(&target.key, "key", target)?,
            need(&target.key_id, "key_id", target)?,
            need(&target.team_id, "team_id", target)?,
        )?;
        let mut req = self
            .http
            .post(&url)
            .header("authorization", format!("bearer {jwt}"))
            .json(&push.payload);
        for (k, v) in &push.headers {
            req = req.header(k, v);
        }
        let resp = req.send().map_err(|e| format!("APNs: {e}"))?;
        let status = resp.status().as_u16();
        let id = resp
            .headers()
            .get("apns-id")
            .and_then(|h| h.to_str().ok())
            .map(str::to_string);
        let v: Value = resp.json().unwrap_or(Value::Null);
        out.id = id;
        if status == 200 {
            out.status = Some(status);
            return Ok(out);
        }
        let code = v["reason"].as_str().unwrap_or("HTTP_ERROR").to_string();
        Ok(out.rejected(Some(status), code, None))
    }
}

/// An OAuth access token for FCM from a service account key or `gcloud` user credentials.
fn google_token(http: &Client, path: &PathBuf) -> Result<GoogleAuth, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!("{}: {e}. Run `gcloud auth application-default login`, or set `credentials` to a service account key", path.display())
    })?;
    let c: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let field = |k: &str| {
        c[k].as_str()
            .ok_or_else(|| format!("{}: no `{k}`", path.display()))
    };
    let (url, form, user) = match c["type"].as_str() {
        Some("service_account") => {
            let token_uri = c["token_uri"]
                .as_str()
                .unwrap_or(GOOGLE_TOKEN_URL)
                .to_string();
            let iat = now();
            let claims = json!({ "iss": field("client_email")?, "scope": FCM_SCOPE, "aud": token_uri, "iat": iat, "exp": iat + 3600 });
            let key = EncodingKey::from_rsa_pem(field("private_key")?.as_bytes())
                .map_err(|e| format!("{}: private_key: {e}", path.display()))?;
            let jwt = jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
                .map_err(|e| e.to_string())?;
            (
                token_uri,
                vec![
                    (
                        "grant_type",
                        "urn:ietf:params:oauth:grant-type:jwt-bearer".to_string(),
                    ),
                    ("assertion", jwt),
                ],
                false,
            )
        }
        Some("authorized_user") => (
            env_or("PUSHBENCH_GOOGLE_TOKEN_URL", GOOGLE_TOKEN_URL),
            vec![
                ("grant_type", "refresh_token".to_string()),
                ("client_id", field("client_id")?.to_string()),
                ("client_secret", field("client_secret")?.to_string()),
                ("refresh_token", field("refresh_token")?.to_string()),
            ],
            true,
        ),
        other => {
            return Err(format!(
                "{}: credentials of type {other:?} are not supported; use a service account key or gcloud user credentials",
                path.display()
            ));
        }
    };
    let resp = http
        .post(&url)
        .form(&form)
        .send()
        .map_err(|e| format!("Google sign-in: {e}"))?;
    let v: Value = resp.json().map_err(|e| format!("Google sign-in: {e}"))?;
    let token = v["access_token"].as_str().ok_or_else(|| {
        let why = v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("no access token");
        format!("Google sign-in failed ({}): {why}. For gcloud credentials, run `gcloud auth application-default login` again", path.display())
    })?;
    let project = c["project_id"]
        .as_str()
        .or(c["quota_project_id"].as_str())
        .map(str::to_string);
    Ok(GoogleAuth {
        token: token.to_string(),
        project,
        user,
    })
}

/// The provider token APNs expects: an ES256 JWT signed with the `.p8` key.
pub fn apns_jwt(key_path: &str, key_id: &str, team_id: &str) -> Result<String, String> {
    let pem = std::fs::read(key_path).map_err(|e| format!("{key_path}: {e}"))?;
    let key = EncodingKey::from_ec_pem(&pem)
        .map_err(|e| format!("{key_path}: not an APNs .p8 key ({e})"))?;
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(key_id.to_string());
    jsonwebtoken::encode(&header, &json!({ "iss": team_id, "iat": now() }), &key)
        .map_err(|e| e.to_string())
}

/// Checks that credentials can be loaded and, for FCM, exchanged for a token.
pub fn check_google(path: &PathBuf) -> Result<String, String> {
    let auth = google_token(&Sender::new(false, false)?.http, path)?;
    Ok(match auth.project {
        Some(p) => format!(
            "signed in ({}, project {p})",
            if auth.user {
                "gcloud user"
            } else {
                "service account"
            }
        ),
        None => "signed in".into(),
    })
}
