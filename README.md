# pushbench

Test push notifications while you develop. Write a push once as a template in your repo,
then send it to the iOS Simulator, a real iPhone through APNs, or Android and iOS through
FCM, from one command:

```bash
pushbench send chat --to sim --to pixel --set name=Ada
```

It works for any app: Flutter, native iOS and Android, React Native. It is one binary for
macOS, Linux and Windows (the iOS Simulator part needs macOS with Xcode).

## Why

Testing a push usually means building for staging, or asking the backend to send one.
pushbench sends it from your machine, with the payload you choose, while the app runs in
a debug build.

| | simulator apps | APNs desktop apps | FCM CLIs | pushbench |
|---|---|---|---|---|
| iOS Simulator without credentials | yes | some | no | yes |
| Real iOS devices through APNs | no | yes | no | yes |
| Android and iOS through FCM HTTP v1 | no | some | yes | yes |
| FCM with `gcloud` credentials (no key file) | no | no | no | yes |
| Templates in the repo, with variables | no | no | config only | yes |
| One template for every platform | no | no | no | yes |
| Lint that knows each platform's rules | no | some | no | yes |
| Plain-language errors with a fix | no | some | no | yes |
| JSON output and exit codes for CI | no | rare | yes | yes |
| Runs on Linux and Windows | no | rare | yes | yes |

## Install

```bash
cargo install --git https://github.com/saimskywalker/pushbench
```

Homebrew and prebuilt binaries come with the first release.

## Quick start

```bash
cd your-app
pushbench init        # pushbench.toml, pushbench.local.toml (git-ignored), push/hello.yaml
```

Set `bundle_id` in `pushbench.toml`, run the app on a booted simulator, allow
notifications, then:

```bash
pushbench send hello --to sim
```

## Templates

A template is one push, written as an [FCM HTTP v1 `message`](https://firebase.google.com/docs/reference/fcm/rest/v1/projects.messages)
(the shape most backends already send), with `{{placeholders}}`:

```yaml
# push/chat.yaml
description: A chat message; tapping it should open the thread.
params:
  sender: Ada            # defaults; override with --set sender=Bo
  unread: 1
message:
  notification:
    title: "{{sender}}"
    body: Sent you a message
  data:
    type: chat_message
    thread_id: "{{uuid}}"
  android:
    priority: high
    notification: { channel_id: messages }
  apns:
    payload:
      aps: { sound: default, badge: "{{unread}}" }
```

- `{{name}}` takes a value from `params` or `--set name=value`. A string that is exactly
  one placeholder keeps the value's type, so `badge` above is a number.
- `{{env.NAME}}` reads an environment variable, `{{uuid}}` is a new UUID and `{{unix}}`
  the current UNIX time.
- `--title` and `--body` replace the notification text, or send a quick push without a
  template: `pushbench send --title Hi --body Test --to sim`.

### What each platform gets

The same template is rendered the way FCM itself delivers it:

- **FCM**: the message as written, plus the token, topic or condition.
- **APNs and the simulator**: `notification` becomes `aps.alert`, `data` keys move to
  the top level, and an image sets `mutable-content`. The push type is worked out from
  the payload (an `event` means a Live Activity, no alert means a background push), the
  topic gets its suffix (`.voip`, `.push-type.liveactivity`, `.complication`, ...),
  background pushes get priority 5 and Live Activities a `timestamp`.
- **Firebase apps**: `gcm.message_id` is added, without which the Firebase iOS SDK
  (and so `firebase_messaging` in Flutter) drops a push sent outside FCM. Turn it off
  with `fcm_compat = false`.

See it for yourself: `pushbench render chat --for apns`, `--for sim` or `--for fcm`.

## Targets

`--to` takes a name from `[targets]`, or one of these; repeat it to send to several.

| Target | Sends through | Needs |
|---|---|---|
| `sim`, `sim:<name or UDID>` | `xcrun simctl push` | macOS with Xcode, `bundle_id` |
| `fcm:<token>` | FCM HTTP v1 | `project`, Google credentials |
| `topic:<name>`, `condition:<expr>` | FCM HTTP v1 | `project`, Google credentials |
| `apns:<token>` | APNs (HTTP/2, token auth) | `.p8` key, `key_id`, `team_id`, `bundle_id` |

FCM reaches Android emulators with Google Play, Android devices, iOS devices, and iOS
Simulators on Apple silicon Macs.

## Configuration

`pushbench.toml` is committed and holds no secrets. `pushbench.local.toml`, next to it and
git-ignored, holds tokens and key paths and overrides the first. Both accept
`{{env.NAME}}`.

```toml
templates = "push"
bundle_id = "com.example.app"   # iOS
package = "com.example.app"     # Android, for --state on adb devices

[fcm]
project = "my-app-dev"          # the Firebase project your dev build uses
# credentials = "~/keys/firebase-dev.json"   # default: gcloud application default credentials

[apns]
environment = "sandbox"         # debug builds: sandbox; TestFlight and App Store: production
# key, key_id, team_id: in pushbench.local.toml

[targets]
sim = "sim"
pixel = { to = "fcm:{{env.PIXEL_TOKEN}}", adb = "emulator-5554" }
iphone = "apns:{{env.IPHONE_TOKEN}}"
```

A target's settings come from the top level, then its section (`[fcm]`, `[apns]`), then
the target itself.

### Credentials

- **FCM**: by default the Google application default credentials, so
  `gcloud auth application-default login` is enough if your account may send through the
  project. Or set `credentials` to a service account key.
- **APNs**: an auth key (`.p8`) from Certificates, Identifiers & Profiles > Keys, with its
  key id and your team id.

Credentials are read locally and sent only to Google's and Apple's servers. Tokens never
go into the history file.

## Commands

| Command | What it does |
|---|---|
| `send [template] --to ...` | Send. `--set k=v`, `--title`, `--body`, `--state foreground\|background\|killed`, `--dry-run` (print, don't send), `--validate` (FCM checks without delivering) |
| `render <template> --for apns\|sim\|fcm` | Print the exact payload |
| `lint [templates]` | Check every template against FCM and APNs rules |
| `explain [code]` | What an FCM, APNs or simulator error means and how to fix it |
| `link <url> --on sim\|adb[:serial]` | Open a deep link |
| `targets` | Configured targets, booted simulators, adb devices |
| `doctor` | Check tools, config, templates and credentials |
| `history [--resend N]` | Recent sends; send one again |
| `init` | Start a config and an example template |

Add `--json` to any command for machine-readable output.

`--state` puts the app in the background or kills it first, so you can test what a tap
does on a cold start. It works on simulators and on FCM targets that name an `adb`
device. On Android the app is killed the way the system would kill it, not
force-stopped: a force-stopped app gets no FCM messages until it is opened again.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Everything was sent |
| 1 | A service rejected a push (bad token, wrong environment, ...) |
| 2 | Bad input: unknown target, missing value, lint errors |
| 3 | Could not send: credentials, network, or a missing tool |

## In CI

```bash
pushbench lint --json                          # fail the build on a broken template
pushbench send chat --to topic:qa --validate   # FCM checks it, nothing is delivered
```

## What it can't do

- Deliver to an Android emulator or device without Google's servers. FCM is the only
  supported way into an app's messaging service.
- Show images on iOS unless the app has a Notification Service Extension.
- Make `simctl push` check APNs headers: the simulator ignores them. Use an FCM or APNs
  target for that.

## Plan

Next: a local web UI, delivery confirmation from device logs, token discovery,
certificate (`.p12`) auth, test suites with JUnit output, and an MCP server for coding
agents. Follow the `plan` issue for details.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Agents: [AGENTS.md](AGENTS.md).

## License

MIT
