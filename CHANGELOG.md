# Changelog

## Unreleased

First version:

- `send` to the iOS Simulator (`simctl push`), FCM HTTP v1 (tokens, topics, conditions)
  and APNs (`.p8` token auth), from one template.
- FCM sign-in with a service account key or `gcloud` application default credentials.
- Templates as FCM v1 messages with `{{params}}`, `{{env.NAME}}`, `{{uuid}}`, `{{unix}}`.
- APNs rendering that matches FCM: push type, topic suffix and priority worked out from
  the payload, Live Activity timestamps, `gcm.message_id` for Firebase apps.
- `render`, `lint`, `explain`, `link`, `targets`, `doctor`, `history`, `init`;
  `--state`, `--dry-run`, `--validate` and `--json`.
