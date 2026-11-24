# AGENTS.md

Rules for AI coding agents working in this repository. People should read
[CONTRIBUTING.md](CONTRIBUTING.md); everything here applies to them too. The
`PR rules` check enforces the rules marked (checked); a pull request that
breaks one cannot be merged.

## What pushbench is

A CLI that sends test push notifications from templates: one FCM v1 `message`
per template (`src/template.rs`), rendered for APNs, the iOS Simulator and FCM
(`src/render.rs`), checked against platform rules (`src/lint.rs`) and sent
through `xcrun simctl push`, FCM HTTP v1 or APNs (`src/send.rs`). Targets and
settings come from `pushbench.toml` and `pushbench.local.toml`
(`src/config.rs`); error codes are explained from one table
(`src/explain.rs`); simulator and adb helpers are in `src/device.rs`; the
commands are in `src/main.rs`.

Read before you touch:

- rendering or lint: "What each platform gets" in README.md, and the tests in
  `src/render.rs` and `src/lint.rs`.
- sending: `tests/cli.rs`, which runs the binary against a local server
  standing in for Google and Apple.
- anything with credentials: SECURITY.md.

## Commands

Use the stable Rust toolchain with rustfmt and clippy. Every one of these must
pass before you open or update a pull request. CI runs the same ones on Linux,
macOS and Windows:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Run one test while iterating: `cargo test --locked live_activities`. See what a
template becomes without sending: `cargo run -- render <template> --for apns`.

## Stop and ask the human

Some things must come from the real world. An agent must never invent them.
Stop and ask the person you work for to provide them:

- **Platform rules.** A new push type, header, size limit, payload key, lint
  rule or error explanation follows Apple's or Google's documentation. Ask for
  the link, or find it and quote it in the pull request. Never write a rule,
  limit or error code from memory.
- **Device behaviour.** What iOS, Android or the Firebase SDKs do with a push
  (what is shown, dropped or delayed) needs a reproduction on a real device or
  simulator by the human. Never claim it from memory.
- **Credentials and tokens.** Never ask for, use or paste real keys, service
  account files or device tokens. Tests use the local server in `tests/cli.rs`
  and keys generated at test time.
- **An accepted issue.** If the change has no issue labelled `accepted`, stop
  and ask the human to open one and wait for a maintainer.

## Code

- **English only** for code, identifiers, comments, commit messages and docs.
  Answer the person you work for in their language.
- Target Rust 1.89 (`rust-version` in `Cargo.toml`), edition 2024. CI builds on
  stable, so a newer feature passes CI and still breaks 1.89; do not use one.
- No panics outside tests: return an error. `unwrap` and `expect` belong in
  tests only.
- Errors say what to do next, in plain words.
- Keep data in tables (`TOPIC_SUFFIX`, the error table) rather than repeated
  branches, and reuse the one rendering path for every target.
- Comments are sparse: only where the code is not self-evident. Do not add
  comments or doc comments to code you did not change.

## Security

- Never print, log or store a credential, OAuth token or device token. The
  history keeps target names and masks inline tokens (`history::mask`).
- Never send a credential to any host but Google's and Apple's push and sign-in
  endpoints (the `PUSHBENCH_*_URL` overrides exist for tests).
- No telemetry, no network calls a command does not need.

## Tests (checked)

- Every change to `src/` comes with a test: a new `#[test]` or a change under
  `tests/`. The check looks for an added `#[test]` line or a changed `tests/`
  file. A bug fix has a test that fails without the fix; check that it does
  before you open the pull request.

## Never

- Never silence rustc, clippy or rustfmt (checked): no new `#[allow(`,
  `#![allow(`, `#[expect(`, `#[rustfmt::skip]`, no lint set to `"allow"` in
  `Cargo.toml`, no `clippy.toml` or `rustfmt.toml`. Fix the finding instead.
- Never delete, skip (`#[ignore]`), loosen or rewrite an existing test or
  expected value to make CI pass. If a test looks wrong, say so in the issue.
- Never rename or remove a command, flag, exit code, config key, target
  scheme, template field or JSON output field. Scripts and CI jobs depend on
  them.
- Never bump the version in `Cargo.toml`, publish, tag or release.
- Never add a dependency or change CI workflows unless the issue asks for it.
- Never add unrelated "improvements": no drive-by refactors, renames,
  reformatting of untouched code or dead code.
- Never open pull requests, issues or comments on your own. The human reviews
  your work and posts it.

## Pull requests (checked)

- First an issue, labelled `accepted` by a maintainer. The pull request says
  `Closes #N`.
- One issue per pull request, at most 400 changed lines.
- At most 2 open pull requests per person.
- Fill in the pull request template, including `### AI use` with the tool and
  model.
- The person submitting has read, run and understood every line, and writes
  the description in their own words. Pull requests that are raw, unreviewed
  agent output will be closed.
