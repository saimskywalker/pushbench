# Contributing

Contributions are welcome. A few things worth knowing before you spend time.

## How changes land

Every change reaches `main` through a pull request that the maintainer reviews
and merges. That includes changes from forks, which is the normal path: you
cannot push to this repository directly, and nothing merges without a review.
Fork the repository, branch from the latest `main`, and open the PR against
`main`. Keep one topic per PR. If a PR sits without a response, a nudge is
fine.

Commit messages follow the short conventional style already in the log:
`fix: ...`, `docs: ...`, `chore: ...`, with the PR number added on merge.

## Before opening a PR

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

CI runs exactly these on Linux, macOS and Windows with the stable toolchain,
and warnings are denied.

## Pull requests

Every CI check must pass before a pull request can be merged. The `PR rules`
check enforces these:

- **Issue first.** Open an issue and wait until a maintainer labels it
  `accepted`, then write `Closes #N` in the pull request.
- **Tests.** Every change to `src/` needs a test: a new `#[test]` or a
  change under `tests/`. A bug fix needs a test that fails without the fix.
- **Small.** At most 400 changed lines (`Cargo.lock` not counted), one issue
  per pull request.
- **No suppressions.** No `#[allow(`, `#[expect(` or `#[rustfmt::skip]`, no
  lint set to `"allow"` in `Cargo.toml`, no `clippy.toml` or `rustfmt.toml`.
  Fix the finding.
- **At most 2 open pull requests** per person.

If a rule really does not fit your change, say why in the description. A
maintainer can waive it with a label (`no-tests-needed`, `large-change` or
`suppression-ok`).

## Legal notice

By submitting a contribution you confirm that you wrote it, that you have the
right to submit it, and that it may be distributed under the project licence
(see [LICENSE-MIT](LICENSE-MIT)).

## AI-assisted contributions

Using AI tools is fine; please just be open about it.

- Say in the PR description which AI tools you used, if any: none, some or most, plus the tool or agent and the model (for example "Claude Code, Opus 5.5" or "Copilot, GPT-5").
- Please write the PR description in your own words; short and plain is fine. It shows us what you understood about the change. Please don't paste generated text.
- Read, run and test every line you submit, and be ready to explain it in review. If you don't understand what a line does, don't submit it.
- Keep the change focused. Agents like to add unrelated "improvements", long comments and dead code; remove them before you open the pull request.
- Pull requests that are raw, unreviewed agent output will be closed.
- If you use an agent, point it at [AGENTS.md](AGENTS.md).
- If the disclosure is missing, a check adds a reminder to the PR. It does not fail the PR, but a maintainer will ask about it before merging.

## What a good change looks like

A test that pins the behaviour down. Rendering and lint rules have unit tests
next to the code; anything that talks to Google or Apple is tested in
`tests/cli.rs`, which runs the binary against a local server and checks the
exact request. For a bug fix, the test fails without the fix.

## Things to be careful with

**Follow the documentation.** A new push type, header, limit, lint rule or
error explanation links to Apple's or Google's documentation in the pull
request. Platform behaviour changes between OS releases, and guesses end up in
other people's test results.

**Credentials.** pushbench handles FCM service account keys, Google user
credentials and APNs keys. Nothing new may print, log or store them, or a
device token, and they go only to Google's and Apple's endpoints. Never put a
real key or token in a test, a fixture or an issue.

**One template, every platform.** A template is an FCM v1 message and every
target is rendered from it. Extend that path rather than adding a second
template format.

**Stable interface.** Commands, flags, exit codes, config keys, target schemes
and JSON output are used in scripts and CI. Add to them; do not rename or
remove them.

## Scope

A tool for testing pushes during development and in CI. It is not a server for
sending pushes to users in production, and it does not store or manage device
tokens beyond the targets you configure.

## Stay in touch

Don't forget to follow [@saimskywalker](https://github.com/saimskywalker) on
GitHub to see new releases and other open-source projects as they come out.
