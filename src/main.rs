//! pushbench: test push notifications on iOS simulators, Android and iOS devices, through
//! `simctl`, FCM and APNs, from templates kept in your repo.

mod config;
mod device;
mod explain;
mod history;
mod lint;
mod render;
mod send;
mod template;

use clap::{Args, Parser, Subcommand, ValueEnum};
use config::{Config, Kind, Target};
use device::{AppState, Device};
use lint::{Finding, Level};
use render::FcmTo;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use template::Template;

// Exit codes: 0 sent, 1 a service rejected a push, 2 bad input or lint errors,
// 3 could not send (credentials, network or a missing tool).
const REJECTED: u8 = 1;
const INVALID: u8 = 2;
const FAILED: u8 = 3;

type Res = Result<u8, (u8, String)>;

#[derive(Parser)]
#[command(
    version,
    about = "Test push notifications: iOS Simulator, APNs and FCM from one template"
)]
struct Cli {
    /// Print machine-readable JSON
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Send a push from a template, or a quick one with --title and --body
    Send(SendArgs),
    /// Print what a template becomes for one way of delivering it
    Render {
        /// Template name in the templates folder, or a path
        template: String,
        #[arg(long = "for", value_enum, default_value = "apns")]
        via: Via,
        /// Set a template value: --set name=value
        #[arg(long = "set", short, value_parser = parse_set)]
        set: Vec<(String, String)>,
    },
    /// Check templates against FCM and APNs rules (every template by default)
    Lint { templates: Vec<String> },
    /// Explain an FCM, APNs or simulator error code (all codes without one)
    Explain { code: Option<String> },
    /// Open a deep link on a simulator or an Android device
    Link {
        url: String,
        /// sim, sim:<device>, adb or adb:<serial>
        #[arg(long, default_value = "sim")]
        on: String,
    },
    /// List configured targets, booted simulators and adb devices
    Targets,
    /// Check the tools, config, templates and credentials
    Doctor,
    /// Show recent sends, or send one again
    History {
        /// Send entry N again (1 is the latest)
        #[arg(long)]
        resend: Option<usize>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Create pushbench.toml and an example template in this folder
    Init,
}

#[derive(Args, Clone, Default)]
struct SendArgs {
    /// Template name in the templates folder, or a path
    template: Option<String>,
    /// Where to send; repeat for several. A name from [targets], or sim, sim:<device>,
    /// fcm:<token>, topic:<name>, condition:<expr>, apns:<token>
    #[arg(long, short, required = true)]
    to: Vec<String>,
    /// Set a template value: --set name=value
    #[arg(long = "set", short, value_parser = parse_set)]
    set: Vec<(String, String)>,
    /// Notification title (replaces the template's)
    #[arg(long)]
    title: Option<String>,
    /// Notification body (replaces the template's)
    #[arg(long)]
    body: Option<String>,
    /// Put the app in this state first (simulators, and FCM targets with `adb`)
    #[arg(long, value_enum)]
    state: Option<AppState>,
    /// Print what would be sent, without sending
    #[arg(long)]
    dry_run: bool,
    /// FCM: have FCM check the message without delivering it
    #[arg(long)]
    validate: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Via {
    Apns,
    Sim,
    Fcm,
}

fn parse_set(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.trim().is_empty() => Ok((k.trim().to_string(), v.to_string())),
        _ => Err(format!("`{s}`: use name=value")),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = std::env::current_dir()
        .map_err(|e| e.to_string())
        .and_then(|d| Config::load(&d))
        .map_err(|e| (INVALID, e))
        .and_then(|config| match cli.command {
            Command::Send(args) => send(&config, &args, cli.json),
            Command::Render { template, via, set } => render_cmd(&config, &template, via, &set),
            Command::Lint { templates } => lint_cmd(&config, &templates, cli.json),
            Command::Explain { code } => explain_cmd(code.as_deref(), cli.json),
            Command::Link { url, on } => {
                let device = Device::parse(&on).map_err(|e| (INVALID, e))?;
                device::open_link(&device, &url).map_err(|e| (FAILED, e))?;
                println!("opened {url} on {on}");
                Ok(0)
            }
            Command::Targets => targets_cmd(&config, cli.json),
            Command::Doctor => doctor(&config),
            Command::History { resend, limit } => history_cmd(&config, resend, limit, cli.json),
            Command::Init => init(&config),
        });
    ExitCode::from(result.unwrap_or_else(|(code, msg)| {
        if cli.json {
            println!("{}", json!({ "error": msg }));
        } else {
            eprintln!("error: {msg}");
        }
        code
    }))
}

fn find_template(config: &Config, name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(name);
    if path.is_file() {
        return Ok(path);
    }
    let dir = config.templates_dir();
    ["yaml", "yml", "json"]
        .iter()
        .map(|ext| dir.join(format!("{name}.{ext}")))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!(
                "no template `{name}`: not a file, and not in {}",
                dir.display()
            )
        })
}

fn message(config: &Config, args: &SendArgs) -> Result<Value, String> {
    let mut message = match &args.template {
        Some(name) => Template::load(&find_template(config, name)?)?.render(&args.set)?,
        None => json!({}),
    };
    for (key, value) in [("title", &args.title), ("body", &args.body)] {
        if let Some(v) = value {
            message["notification"][key] = v.clone().into();
        }
    }
    match message.as_object() {
        Some(o) if !o.is_empty() => Ok(message),
        _ => Err("give a template, or --title and --body".into()),
    }
}

fn print_findings(findings: &[Finding], file: Option<&Path>) {
    for f in findings {
        let level = if f.level == Level::Error {
            "error"
        } else {
            "warning"
        };
        let file = file.map_or(String::new(), |p| format!("{}: ", p.display()));
        eprintln!("{level}: {file}{}: {}", f.at, f.message);
    }
}

fn send(config: &Config, args: &SendArgs, json_out: bool) -> Res {
    let message = message(config, args).map_err(|e| (INVALID, e))?;
    let findings = lint::lint(&message);
    if !json_out {
        print_findings(&findings, None);
    }
    if findings.iter().any(|f| f.level == Level::Error) {
        if json_out {
            println!("{}", json!({ "lint": findings, "results": [] }));
        }
        return Ok(INVALID);
    }
    let targets = args
        .to
        .iter()
        .map(|spec| config.target(spec))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| (INVALID, e))?;
    let mut sender = send::Sender::new(args.dry_run, args.validate).map_err(|e| (FAILED, e))?;
    let mut code = 0;
    let mut results = Vec::new();
    for mut target in targets {
        target.name = history::mask(&target.name);
        let outcome = match args.state.filter(|_| !args.dry_run) {
            Some(state) => prepare(&target, state).and_then(|()| sender.send(&target, &message)),
            None => sender.send(&target, &message),
        };
        let result = match outcome {
            Ok(o) => {
                if !o.ok {
                    code = code.max(REJECTED);
                }
                serde_json::to_value(&o).unwrap_or_default()
            }
            Err(e) => {
                code = FAILED;
                json!({ "target": target.name, "ok": false, "error": e })
            }
        };
        if !json_out {
            print_result(&result);
        }
        results.push(result);
    }
    if !args.dry_run {
        let entry = history::Entry {
            time: now(),
            template: args.template.clone(),
            sets: args.set.clone(),
            title: args.title.clone(),
            body: args.body.clone(),
            to: args.to.iter().map(|t| history::mask(t)).collect(),
            ok: code == 0,
        };
        if let Err(e) = history::append(&entry) {
            eprintln!("warning: history not saved: {e}");
        }
    }
    if json_out {
        println!("{}", json!({ "lint": findings, "results": results }));
    }
    Ok(code)
}

fn print_result(r: &Value) {
    let target = r["target"].as_str().unwrap_or("?");
    let via = r["via"]
        .as_str()
        .map_or(String::new(), |v| format!(" ({v})"));
    if let Some(request) = r.get("request") {
        println!(
            "→ {target}{via} would get:\n{}",
            serde_json::to_string_pretty(request).unwrap_or_default()
        );
    } else if r["ok"] == true {
        let id = r["id"]
            .as_str()
            .map_or(String::new(), |i| format!("  id {i}"));
        println!("✓ {target}{via}  sent{id}");
    } else if let Some(e) = r["error"].as_str() {
        println!("✗ {target}  could not send: {e}");
    } else {
        let status = r["status"]
            .as_u64()
            .map_or(String::new(), |s| format!(" (HTTP {s})"));
        println!(
            "✗ {target}{via}  {}{status}",
            r["code"].as_str().unwrap_or("rejected")
        );
        if let Some(x) = r.get("explanation") {
            println!(
                "    {}\n    fix: {}",
                x["meaning"].as_str().unwrap_or(""),
                x["fix"].as_str().unwrap_or("")
            );
        }
        if let Some(d) = r["detail"].as_str() {
            println!("    {d}");
        }
    }
}

/// Gets the app into `state` on the device behind `target`.
fn prepare(target: &Target, state: AppState) -> Result<(), String> {
    let (device, id) = match &target.kind {
        Kind::Sim => (Device::Sim(target.address.clone()), target.bundle_id.clone()),
        Kind::Fcm(_) => match &target.adb {
            Some(serial) => (Device::Adb((!serial.is_empty()).then(|| serial.clone())), target.package.clone()),
            None => return Err(format!("--state on `{}` needs `adb = \"<serial>\"` on the target", target.name)),
        },
        Kind::Apns => return Err("--state works on simulators and adb devices; on an iPhone, set the app's state by hand".into()),
    };
    let id =
        id.ok_or("--state needs `bundle_id` (iOS) or `package` (Android) in pushbench.toml")?;
    device::set_state(&device, &id, state)?;
    std::thread::sleep(std::time::Duration::from_millis(1500));
    Ok(())
}

fn render_cmd(config: &Config, name: &str, via: Via, set: &[(String, String)]) -> Res {
    let message = Template::load(&find_template(config, name).map_err(|e| (INVALID, e))?)
        .and_then(|t| t.render(set))
        .map_err(|e| (INVALID, e))?;
    let sim = config.target("sim").map_err(|e| (INVALID, e))?;
    let bundle = sim.bundle_id.unwrap_or_else(|| "com.example.app".into());
    let out = match via {
        Via::Apns => {
            let p = render::apns(&message, &bundle, sim.fcm_compat);
            let headers: serde_json::Map<String, Value> =
                p.headers.into_iter().map(|(k, v)| (k, v.into())).collect();
            json!({ "headers": headers, "payload": p.payload })
        }
        Via::Sim => render::apns(&message, &bundle, sim.fcm_compat).payload,
        Via::Fcm => render::fcm(&message, &FcmTo::Token("<token>".into()), false),
    };
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    Ok(0)
}

fn lint_cmd(config: &Config, names: &[String], json_out: bool) -> Res {
    let paths: Vec<PathBuf> = if names.is_empty() {
        let dir = config.templates_dir();
        let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|e| (INVALID, format!("{}: {e}", dir.display())))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|x| x == "yaml" || x == "yml" || x == "json")
            })
            .collect();
        found.sort();
        found
    } else {
        names
            .iter()
            .map(|n| find_template(config, n))
            .collect::<Result<_, _>>()
            .map_err(|e| (INVALID, e))?
    };
    if paths.is_empty() {
        return Err((
            INVALID,
            format!("no templates in {}", config.templates_dir().display()),
        ));
    }
    let mut report = Vec::new();
    let mut errors = 0;
    for path in &paths {
        let mut description = None;
        let findings = match Template::load(path) {
            Err(e) => vec![Finding {
                level: Level::Error,
                at: "template".into(),
                message: e,
            }],
            Ok(t) => {
                description = t.description.clone();
                let (message, missing) = t.render_partial(&[]);
                let mut f = lint::lint(&message);
                f.extend(missing.into_iter().map(|m| Finding {
                    level: Level::Warning,
                    at: format!("{{{{{m}}}}}"),
                    message: "has no default: pass --set or set the environment variable when sending".into(),
                }));
                f
            }
        };
        errors += findings.iter().filter(|f| f.level == Level::Error).count();
        if !json_out {
            if findings.is_empty() {
                println!(
                    "✓ {}{}",
                    path.display(),
                    description
                        .as_ref()
                        .map_or(String::new(), |d| format!("  {d}"))
                );
            } else {
                print_findings(&findings, Some(path));
            }
        }
        report.push(json!({ "template": path, "description": description, "findings": findings }));
    }
    if json_out {
        println!("{}", json!(report));
    }
    Ok(if errors > 0 { INVALID } else { 0 })
}

fn explain_cmd(code: Option<&str>, json_out: bool) -> Res {
    let list: Vec<_> = match code {
        Some(c) => vec![explain::explain(c).ok_or((
            INVALID,
            format!("unknown code `{c}`; `pushbench explain` lists them"),
        ))?],
        None => explain::all().iter().collect(),
    };
    if json_out {
        println!("{}", json!(list));
    } else {
        for x in list {
            let retry = if x.retryable {
                "retryable"
            } else {
                "not retryable"
            };
            println!("{} ({retry})\n  {}\n  fix: {}", x.code, x.meaning, x.fix);
        }
    }
    Ok(0)
}

fn targets_cmd(config: &Config, json_out: bool) -> Res {
    let aliases: Vec<(String, String)> = config
        .aliases()
        .into_iter()
        .map(|(k, v)| (k, history::mask(&v)))
        .collect();
    let sims = device::booted_sims();
    let adb = device::adb_devices();
    if json_out {
        println!(
            "{}",
            json!({
                "targets": aliases.iter().map(|(k, v)| json!({ "name": k, "to": v })).collect::<Vec<_>>(),
                "simulators": sims.iter().map(|(n, u, r)| json!({ "name": n, "udid": u, "runtime": r })).collect::<Vec<_>>(),
                "adb": adb.iter().map(|(s, st)| json!({ "serial": s, "state": st })).collect::<Vec<_>>(),
            })
        );
        return Ok(0);
    }
    println!("targets in pushbench.toml:");
    if aliases.is_empty() {
        println!("  (none)");
    }
    for (k, v) in &aliases {
        println!("  {k:<14} {v}");
    }
    println!("booted simulators (--to sim:<name or udid>):");
    if sims.is_empty() {
        println!("  (none)");
    }
    for (name, udid, runtime) in &sims {
        println!("  {name} ({runtime})  {udid}");
    }
    println!("adb devices:");
    if adb.is_empty() {
        println!("  (none)");
    }
    for (serial, state) in &adb {
        println!("  {serial}  {state}");
    }
    Ok(0)
}

fn doctor(config: &Config) -> Res {
    let mut problems = 0;
    let mut check = |ok: Option<bool>, what: &str, detail: String| {
        let mark = match ok {
            Some(true) => "✓",
            Some(false) => {
                problems += 1;
                "✗"
            }
            None => "–",
        };
        println!("{mark} {what:<12} {detail}");
    };
    if config.found {
        check(
            Some(true),
            "config",
            config.dir.join(config::FILE).display().to_string(),
        );
    } else {
        check(
            None,
            "config",
            "no pushbench.toml here or above: run `pushbench init`".into(),
        );
    }
    let dir = config.templates_dir();
    match std::fs::read_dir(&dir) {
        Ok(entries) => check(
            Some(true),
            "templates",
            format!("{} in {}", entries.count(), dir.display()),
        ),
        Err(_) => check(None, "templates", format!("no folder {}", dir.display())),
    }
    match device::run("xcrun", &["simctl", "help"], None) {
        Ok(_) => check(
            Some(true),
            "simctl",
            format!("{} booted simulator(s)", device::booted_sims().len()),
        ),
        Err(_) => check(
            None,
            "simctl",
            "not available (the iOS Simulator needs macOS with Xcode)".into(),
        ),
    }
    match device::run("adb", &["version"], None) {
        Ok(_) => check(
            Some(true),
            "adb",
            format!("{} device(s)", device::adb_devices().len()),
        ),
        Err(_) => check(
            None,
            "adb",
            "not found (needed only for Android --state and links)".into(),
        ),
    }
    if config.has("fcm") {
        match config.target("topic:doctor") {
            Ok(t) => {
                let path = t
                    .credentials
                    .as_ref()
                    .map_or_else(send::adc_path, PathBuf::from);
                if t.credentials.is_none() && !path.exists() {
                    check(
                        None,
                        "fcm",
                        "no credentials yet: run `gcloud auth application-default login`, or set `credentials` under [fcm]".into(),
                    );
                } else {
                    match send::check_google(&path) {
                        Ok(msg) => {
                            check(Some(true), "fcm", format!("{msg} via {}", path.display()))
                        }
                        Err(e) => check(Some(false), "fcm", e),
                    }
                }
                if t.project.is_none() {
                    check(
                        None,
                        "fcm",
                        "no `project` under [fcm]: taken from the credentials if they name one"
                            .into(),
                    );
                }
            }
            Err(e) => check(Some(false), "fcm", e),
        }
    } else {
        check(None, "fcm", "not set up (no [fcm] section)".into());
    }
    match config.target("apns:doctor-check-token") {
        Ok(t) if t.key.is_some() || t.key_id.is_some() || t.team_id.is_some() => {
            match (t.key.as_deref(), t.key_id.as_deref(), t.team_id.as_deref()) {
                (Some(key), Some(id), Some(team)) => match send::apns_jwt(key, id, team) {
                    Ok(_) => check(
                        Some(true),
                        "apns",
                        format!(
                            "key {id} (team {team}) signs, {}",
                            t.environment.as_deref().unwrap_or("sandbox")
                        ),
                    ),
                    Err(e) => check(Some(false), "apns", e),
                },
                _ => check(
                    Some(false),
                    "apns",
                    "set all of key, key_id and team_id".into(),
                ),
            }
        }
        Ok(_) => check(None, "apns", "not set up (no key under [apns])".into()),
        Err(e) => check(Some(false), "apns", e),
    }
    for name in config.aliases().keys() {
        if let Err(e) = config.target(name) {
            check(Some(false), "target", e);
        }
    }
    Ok(if problems > 0 { FAILED } else { 0 })
}

fn ago(time: u64) -> String {
    let s = now().saturating_sub(time);
    match s {
        0..60 => format!("{s}s ago"),
        60..3600 => format!("{}m ago", s / 60),
        3600..86400 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

fn history_cmd(config: &Config, resend: Option<usize>, limit: usize, json_out: bool) -> Res {
    let entries = history::read();
    if let Some(n) = resend {
        let entry = entries
            .iter()
            .rev()
            .nth(n.saturating_sub(1))
            .ok_or((INVALID, format!("no history entry {n}")))?;
        if let Some(t) = entry.to.iter().find(|t| t.contains('…')) {
            return Err((
                INVALID,
                format!(
                    "`{t}` was an inline token and history keeps only a masked copy; resend to a [targets] name instead"
                ),
            ));
        }
        let args = SendArgs {
            template: entry.template.clone(),
            to: entry.to.clone(),
            set: entry.sets.clone(),
            title: entry.title.clone(),
            body: entry.body.clone(),
            ..SendArgs::default()
        };
        return send(config, &args, json_out);
    }
    let recent: Vec<_> = entries.iter().rev().take(limit).collect();
    if json_out {
        println!("{}", json!(recent));
        return Ok(0);
    }
    if recent.is_empty() {
        println!("no sends yet");
    }
    for (i, e) in recent.iter().enumerate() {
        let what = e
            .template
            .clone()
            .or_else(|| e.title.clone())
            .unwrap_or_else(|| "(quick)".into());
        let mark = if e.ok { "✓" } else { "✗" };
        println!(
            "{:>3}  {mark} {:<9} {what} → {}",
            i + 1,
            ago(e.time),
            e.to.join(", ")
        );
    }
    Ok(0)
}

const INIT_CONFIG: &str = r#"# pushbench: https://github.com/saimskywalker/pushbench
# Commit this file. Tokens and key paths go in pushbench.local.toml, which git ignores.

templates = "push"
bundle_id = "com.example.app"        # iOS bundle id
package = "com.example.app"          # Android application id (for --state on adb devices)

[fcm]
project = "my-firebase-dev-project"  # the Firebase project your dev build uses
# credentials = "~/keys/firebase-dev.json"  # a service account key; default: `gcloud auth application-default login`

[apns]
environment = "sandbox"              # debug builds: sandbox; TestFlight and App Store builds: production
# key, key_id and team_id go in pushbench.local.toml

[targets]
sim = "sim"                          # the booted iOS Simulator
# pixel = { to = "fcm:{{env.PIXEL_FCM_TOKEN}}", adb = "emulator-5554" }
# iphone = "apns:{{env.IPHONE_APNS_TOKEN}}"
"#;

const INIT_LOCAL: &str = r#"# Not committed: tokens, key paths and anything else specific to this machine.

# [apns]
# key = "~/keys/AuthKey_ABC123DEFG.p8"
# key_id = "ABC123DEFG"
# team_id = "TEAM123456"

# [targets]
# my-phone = "apns:<device token>"
"#;

const INIT_TEMPLATE: &str = r#"description: A chat message; tapping it should open the thread.
params:
  name: Ada
message:
  notification:
    title: "{{name}} sent you a message"
    body: Tap to open the chat.
  data:
    type: chat_message
    thread_id: "{{uuid}}"
  android:
    priority: high
  apns:
    payload:
      aps:
        sound: default
        badge: 1
"#;

fn init(config: &Config) -> Res {
    let dir = std::env::current_dir().map_err(|e| (FAILED, e.to_string()))?;
    if dir.join(config::FILE).exists() {
        return Err((INVALID, format!("{} already exists here", config::FILE)));
    }
    if config.found {
        eprintln!(
            "note: {} also has a pushbench.toml; this one takes over below it",
            config.dir.display()
        );
    }
    let write = |path: &Path, text: &str| -> Result<bool, (u8, String)> {
        if path.exists() {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| (FAILED, e.to_string()))?;
        }
        std::fs::write(path, text).map_err(|e| (FAILED, format!("{}: {e}", path.display())))?;
        Ok(true)
    };
    for (file, text) in [
        (config::FILE, INIT_CONFIG),
        (config::LOCAL_FILE, INIT_LOCAL),
        ("push/hello.yaml", INIT_TEMPLATE),
    ] {
        if write(&dir.join(file), text)? {
            println!("created {file}");
        }
    }
    let ignore = dir.join(".gitignore");
    let current = std::fs::read_to_string(&ignore).unwrap_or_default();
    if !current.lines().any(|l| l.trim() == config::LOCAL_FILE) {
        let sep = if current.is_empty() || current.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        std::fs::write(&ignore, format!("{current}{sep}{}\n", config::LOCAL_FILE))
            .map_err(|e| (FAILED, e.to_string()))?;
        println!("added {} to .gitignore", config::LOCAL_FILE);
    }
    println!(
        "\nnext: set bundle_id, boot a simulator with the app installed, then run\n  pushbench send hello --to sim"
    );
    Ok(0)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
