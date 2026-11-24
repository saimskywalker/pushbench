//! iOS Simulator (`xcrun simctl`) and Android (`adb`) helpers.

use clap::ValueEnum;
use std::io::Write;
use std::process::{Command, Stdio};

/// What the app should be doing when the push arrives.
#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum AppState {
    /// Open and in front.
    Foreground,
    /// Running, but another app is in front.
    Background,
    /// Not running. On Android the process is killed the way the system would, not
    /// force-stopped: a force-stopped app gets no FCM messages until it is opened again.
    Killed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Device {
    Sim(String),
    /// An adb serial; `None` for the only connected device.
    Adb(Option<String>),
}

impl Device {
    /// `sim`, `sim:<name or udid>`, `adb`, `adb:<serial>`.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let (scheme, rest) = spec.split_once(':').unwrap_or((spec, ""));
        match scheme {
            "sim" => Ok(Self::Sim(if rest.is_empty() {
                "booted".into()
            } else {
                rest.into()
            })),
            "adb" => Ok(Self::Adb((!rest.is_empty()).then(|| rest.to_string()))),
            _ => Err(format!(
                "`{spec}`: use sim, sim:<device>, adb or adb:<serial>"
            )),
        }
    }
}

/// Runs a command and returns its stdout, or an error with its stderr.
pub fn run(program: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run `{program}`: {e}"))?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input)
            .map_err(|e| format!("`{program}`: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("`{program}`: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn simctl(args: &[&str], stdin: Option<&[u8]>) -> Result<String, String> {
    let mut all = vec!["simctl"];
    all.extend_from_slice(args);
    run("xcrun", &all, stdin)
}

fn adb(serial: &Option<String>, args: &[&str]) -> Result<String, String> {
    let mut all: Vec<&str> = serial.as_deref().map_or(vec![], |s| vec!["-s", s]);
    all.extend_from_slice(args);
    run("adb", &all, None)
}

/// Booted simulators as (name, udid, runtime).
pub fn booted_sims() -> Vec<(String, String, String)> {
    let Ok(json) = simctl(&["list", "devices", "booted", "-j"], None) else {
        return vec![];
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else {
        return vec![];
    };
    let Some(runtimes) = v["devices"].as_object() else {
        return vec![];
    };
    runtimes
        .iter()
        .flat_map(|(runtime, devices)| {
            let runtime = runtime
                .rsplit('.')
                .next()
                .unwrap_or(runtime)
                .replace('-', " ");
            devices.as_array().into_iter().flatten().map(move |d| {
                (
                    d["name"].as_str().unwrap_or("?").to_string(),
                    d["udid"].as_str().unwrap_or("?").to_string(),
                    runtime.clone(),
                )
            })
        })
        .collect()
}

/// Connected adb devices as (serial, state).
pub fn adb_devices() -> Vec<(String, String)> {
    let Ok(out) = run("adb", &["devices"], None) else {
        return vec![];
    };
    out.lines()
        .skip(1)
        .filter_map(|l| l.split_once('\t'))
        .map(|(s, state)| (s.to_string(), state.trim().to_string()))
        .collect()
}

pub fn push_to_sim(device: &str, bundle_id: &str, payload: &[u8]) -> Result<String, String> {
    simctl(&["push", device, bundle_id, "-"], Some(payload))
}

pub fn open_link(device: &Device, url: &str) -> Result<(), String> {
    match device {
        Device::Sim(d) => simctl(&["openurl", d, url], None).map(drop),
        Device::Adb(s) => adb(
            s,
            &[
                "shell",
                "am",
                "start",
                "-W",
                "-a",
                "android.intent.action.VIEW",
                "-d",
                url,
            ],
        )
        .map(drop),
    }
}

/// Puts the app (`id`: bundle id or Android package) into `state`.
pub fn set_state(device: &Device, id: &str, state: AppState) -> Result<(), String> {
    match (device, state) {
        (Device::Sim(d), AppState::Foreground) => simctl(&["launch", d, id], None).map(drop),
        (Device::Sim(d), AppState::Background) => {
            simctl(&["launch", d, id], None)?;
            simctl(&["launch", d, "com.apple.Preferences"], None).map(drop)
        }
        (Device::Sim(d), AppState::Killed) => {
            // terminate fails when the app is not running, which is the state we want
            let _ = simctl(&["terminate", d, id], None);
            Ok(())
        }
        (Device::Adb(s), AppState::Foreground) => adb(
            s,
            &[
                "shell",
                "monkey",
                "-p",
                id,
                "-c",
                "android.intent.category.LAUNCHER",
                "1",
            ],
        )
        .map(drop),
        (Device::Adb(s), AppState::Background) => {
            adb(
                s,
                &[
                    "shell",
                    "monkey",
                    "-p",
                    id,
                    "-c",
                    "android.intent.category.LAUNCHER",
                    "1",
                ],
            )?;
            adb(s, &["shell", "input", "keyevent", "KEYCODE_HOME"]).map(drop)
        }
        (Device::Adb(s), AppState::Killed) => {
            adb(s, &["shell", "input", "keyevent", "KEYCODE_HOME"])?;
            adb(s, &["shell", "am", "kill", id]).map(drop)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devices() {
        assert_eq!(Device::parse("sim").unwrap(), Device::Sim("booted".into()));
        assert_eq!(
            Device::parse("sim:iPhone 17").unwrap(),
            Device::Sim("iPhone 17".into())
        );
        assert_eq!(Device::parse("adb").unwrap(), Device::Adb(None));
        assert_eq!(
            Device::parse("adb:emulator-5554").unwrap(),
            Device::Adb(Some("emulator-5554".into()))
        );
        assert!(Device::parse("usb").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn run_reports_stderr() {
        let err = run("sh", &["-c", "echo nope >&2; exit 3"], None).unwrap_err();
        assert_eq!(err, "nope");
        assert_eq!(run("sh", &["-c", "cat"], Some(b"in")).unwrap(), "in");
    }
}
