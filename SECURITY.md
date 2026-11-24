# Security policy

## Supported versions

`pushbench` is not yet published to crates.io. Only the latest `0.1.x`
on `main` gets security fixes.

| Version | Supported |
|---|---|
| 0.1.x (`main`) | yes |

## Reporting a vulnerability

Please do not open a public issue for a vulnerability.

Report it privately through GitHub's private vulnerability reporting: open the
[Security tab](https://github.com/saimskywalker/pushbench/security)
and choose "Report a vulnerability". Only the maintainer can see the report.

pushbench handles credentials: FCM service account keys, Google
application default credentials and APNs `.p8` keys. The cases that matter
most are the ones that could leak or misuse them:

- a credential, OAuth token or device token showing up in output, logs,
  history files or error messages
- a request carrying a credential to any host other than Google's and Apple's
  push endpoints
- the local web UI being reachable from another machine, or a web page in the
  browser being able to drive it (it listens on `127.0.0.1` only)
- a template or config making pushbench read or run something outside what
  the user asked for

Please include:

- the smallest command, template or config that shows it (with any real
  keys and tokens removed)
- what happened and what you expected
- the pushbench version (`pushbench --version`) and your OS

You should hear back usually within a week. Once a fix is ready it is merged
and the advisory is published, with credit to you unless you prefer otherwise.
