//! Plain-language meaning and fix for FCM and APNs error codes.

use serde::Serialize;

#[derive(Debug, Serialize, Clone, Copy)]
pub struct Explanation {
    pub code: &'static str,
    pub meaning: &'static str,
    pub fix: &'static str,
    /// Whether sending the same push again later can succeed.
    pub retryable: bool,
}

const fn e(
    code: &'static str,
    meaning: &'static str,
    fix: &'static str,
    retryable: bool,
) -> Explanation {
    Explanation {
        code,
        meaning,
        fix,
        retryable,
    }
}

const TABLE: &[Explanation] = &[
    // FCM HTTP v1 (`errorCode` in the error details, or the status)
    e(
        "UNREGISTERED",
        "The token is no longer valid: the app was uninstalled, its data cleared, or the token was refreshed.",
        "Get a fresh token from the app and send again.",
        false,
    ),
    e(
        "INVALID_ARGUMENT",
        "FCM rejected the message: a malformed token, a wrong field, a data value that is not a string, or a payload over 4 KB.",
        "Run `pushbench lint` on the template and check the token; the message from FCM says which field.",
        false,
    ),
    e(
        "SENDER_ID_MISMATCH",
        "The token belongs to a different Firebase project than the one you sent through.",
        "Send through the project the app's google-services.json / GoogleService-Info.plist comes from, e.g. the dev project for a dev build.",
        false,
    ),
    e(
        "THIRD_PARTY_AUTH_ERROR",
        "FCM could not authenticate with APNs (iOS) or Web Push: the APNs key or certificate in the Firebase project is missing, invalid or for the wrong environment.",
        "Upload a valid APNs auth key (.p8) under Project settings > Cloud Messaging for this Firebase project.",
        false,
    ),
    e(
        "QUOTA_EXCEEDED",
        "You sent too many messages: to the project, to one device, or to one topic.",
        "Wait and retry with backoff.",
        true,
    ),
    e(
        "UNAVAILABLE",
        "FCM is overloaded or briefly unavailable.",
        "Retry with exponential backoff.",
        true,
    ),
    e(
        "INTERNAL",
        "An unknown error inside FCM.",
        "Retry with exponential backoff.",
        true,
    ),
    e(
        "UNAUTHENTICATED",
        "The OAuth token was missing, expired or not accepted.",
        "Check the credentials: `gcloud auth application-default login` or a service account key, then run `pushbench doctor`.",
        false,
    ),
    e(
        "PERMISSION_DENIED",
        "The credentials are valid but may not send through this project.",
        "Give the account the `cloudmessaging.messages.create` permission (e.g. the Firebase Cloud Messaging Admin role) on the project, or use that project's service account.",
        false,
    ),
    e(
        "NOT_FOUND",
        "The project id is wrong, or the Firebase Cloud Messaging API is not enabled for it.",
        "Check `project` in pushbench.toml and enable the FCM API in the Google Cloud console.",
        false,
    ),
    // APNs (`reason` in the response body)
    e(
        "BadDeviceToken",
        "The device token is malformed, or it is for the other APNs environment.",
        "Debug builds use sandbox tokens and release/TestFlight builds production tokens: set `environment` to match the build.",
        false,
    ),
    e(
        "BadCertificateEnvironment",
        "The certificate is for the other APNs environment.",
        "Use a certificate for this environment, or switch `environment`.",
        false,
    ),
    e(
        "DeviceTokenNotForTopic",
        "The token is not for the app named by the topic.",
        "Set `bundle_id` (or `apns-topic`) to the app the token came from.",
        false,
    ),
    e(
        "TopicDisallowed",
        "Pushing to this topic is not allowed.",
        "Check the bundle id and the push type's topic suffix.",
        false,
    ),
    e(
        "BadTopic",
        "The topic is invalid.",
        "Check `bundle_id`; push types such as voip and liveactivity add a suffix automatically.",
        false,
    ),
    e(
        "MissingTopic",
        "The request had no topic and the key is not tied to one app.",
        "Set `bundle_id`.",
        false,
    ),
    e(
        "Unregistered",
        "The token is no longer active for the topic.",
        "Get a fresh token from the app.",
        false,
    ),
    e(
        "ExpiredToken",
        "The device token has expired.",
        "Get a fresh token from the app.",
        false,
    ),
    e(
        "InvalidProviderToken",
        "The signed JWT is invalid: wrong key id, team id or .p8 key, or the key was revoked.",
        "Check `key`, `key_id` and `team_id` against Certificates, Identifiers & Profiles > Keys.",
        false,
    ),
    e(
        "ExpiredProviderToken",
        "The JWT is older than an hour, or the computer's clock is off.",
        "Check the system clock; pushbench signs a fresh JWT for every run.",
        true,
    ),
    e(
        "MissingProviderToken",
        "No provider token or certificate was sent.",
        "Set `key`, `key_id` and `team_id` under [apns].",
        false,
    ),
    e(
        "TooManyProviderTokenUpdates",
        "The JWT was replaced too often.",
        "Wait a few minutes before sending again.",
        true,
    ),
    e(
        "PayloadTooLarge",
        "The payload is over 4 KB (5 KB for VoIP).",
        "Shorten the payload; `pushbench lint` shows the size.",
        false,
    ),
    e(
        "BadPriority",
        "`apns-priority` is not 1, 5 or 10.",
        "Use 10 for alerts and 5 for background pushes.",
        false,
    ),
    e(
        "InvalidPushType",
        "`apns-push-type` is not a valid value.",
        "Use alert, background, voip, liveactivity, complication, fileprovider, location, widgets or mdm.",
        false,
    ),
    e(
        "BadExpirationDate",
        "`apns-expiration` is invalid.",
        "Use a UNIX time in seconds, or 0 to deliver once.",
        false,
    ),
    e(
        "BadCollapseId",
        "`apns-collapse-id` is over 64 bytes.",
        "Shorten the collapse id.",
        false,
    ),
    e(
        "BadMessageId",
        "`apns-id` is not a UUID.",
        "Remove `apns-id` or use a canonical UUID.",
        false,
    ),
    e(
        "PayloadEmpty",
        "The payload was empty.",
        "Add an `aps` dictionary.",
        false,
    ),
    e(
        "Forbidden",
        "The action is not allowed for this key or certificate.",
        "Check the key has the Apple Push Notifications service enabled.",
        false,
    ),
    e(
        "TooManyRequests",
        "Too many pushes to the same device token.",
        "Wait and send less often.",
        true,
    ),
    e(
        "InternalServerError",
        "An internal error at APNs.",
        "Retry later.",
        true,
    ),
    e(
        "ServiceUnavailable",
        "APNs is unavailable.",
        "Retry later.",
        true,
    ),
    e(
        "Shutdown",
        "The APNs server is shutting down.",
        "Retry; a new connection goes to another server.",
        true,
    ),
    // xcrun simctl push
    e(
        "SimulatorNotBooted",
        "No booted simulator matches the target.",
        "Boot one (`open -a Simulator`) or name it: `--to sim:<name or UDID>`; `pushbench targets` lists them.",
        false,
    ),
    e(
        "SimulatorNotAuthorized",
        "The app has not been allowed to show notifications on this simulator.",
        "Open the app on the simulator and tap Allow when it asks for notifications (or turn them on in Settings > Notifications).",
        false,
    ),
    e(
        "SimulatorPushFailed",
        "`simctl push` refused the push, most often because the app is not installed on that simulator.",
        "Install and run the app on the simulator, and check `bundle_id`.",
        false,
    ),
];

/// FCM and APNs share some names in different case (`UNREGISTERED`, `Unregistered`), so an
/// exact match wins over a case-insensitive one.
pub fn explain(code: &str) -> Option<&'static Explanation> {
    TABLE
        .iter()
        .find(|x| x.code == code)
        .or_else(|| TABLE.iter().find(|x| x.code.eq_ignore_ascii_case(code)))
}

pub fn all() -> &'static [Explanation] {
    TABLE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_codes_case_insensitively() {
        assert!(!explain("baddevicetoken").unwrap().retryable);
        assert!(explain("UNAVAILABLE").unwrap().retryable);
        assert!(explain("NoSuchCode").is_none());
    }

    #[test]
    fn exact_case_tells_fcm_and_apns_apart() {
        assert!(
            explain("UNREGISTERED")
                .unwrap()
                .meaning
                .contains("uninstalled")
        );
        assert!(
            explain("Unregistered")
                .unwrap()
                .meaning
                .contains("no longer active for the topic")
        );
    }

    #[test]
    fn codes_are_unique() {
        let mut codes: Vec<_> = TABLE.iter().map(|x| x.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), TABLE.len());
    }
}
