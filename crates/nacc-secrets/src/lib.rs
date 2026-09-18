//! Secret redaction (master plan S13.5, S27.28): the single place text that
//! might reach a durable record — event payloads, log tails, handoffs,
//! diagnostics bundles — goes through before it is stored. Redaction here is
//! value-based and prefix-based: exact configured secret values are replaced
//! wherever they appear, and well-known credential token shapes are replaced
//! even when NACC was never told about them. It is deliberately *not* a
//! general taint tracker: its contract is "these exact strings and these
//! known token shapes never reach disk through NACC."
//!
//! Credential *storage* (Windows Credential Manager for NACC-owned secrets)
//! stays with the user's own credential tools in this build: NACC's adapters
//! use each provider's native store and store only references, so the
//! secrets NACC itself would have to keep do not exist yet. The moment one
//! does, it goes behind [`CredentialReference`] and a store implementation,
//! never into plaintext.

/// A named pointer to a secret kept in the user's own credential store.
/// Cloning this is safe; it is a label, not a secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialReference {
    pub store: &'static str,
    pub key: String,
}

/// The token shapes NACC recognizes on sight. Lowercase match; each entry is
/// (prefix, minimum total length) so short look-alike strings ("sk") are not
/// false positives.
const TOKEN_PREFIXES: &[(&str, usize)] = &[
    ("sk-", 20),
    ("sk-ant-", 20),
    ("ghp_", 36),
    ("gho_", 36),
    ("github_pat_", 30),
    ("xoxb-", 20),
    ("xoxp-", 20),
    ("glpat-", 20),
];

/// Replace every occurrence of every configured secret value in `text`.
/// Returns the redacted text and how many replacements were made, so the
/// caller can log "3 redactions" without logging what was redacted.
pub fn redact_values(text: &str, secret_values: &[String]) -> (String, usize) {
    let mut redacted = text.to_string();
    let mut count = 0usize;
    for secret in secret_values {
        if secret.is_empty() {
            continue;
        }
        while let Some(position) = redacted.find(secret.as_str()) {
            redacted.replace_range(position..position + secret.len(), "[REDACTED]");
            count += 1;
        }
    }
    (redacted, count)
}

/// Replace well-known credential token shapes with a type-preserving marker.
/// Only the prefix plus a length bound is matched — enough for the known
/// shapes, conservative enough to leave ordinary words alone.
pub fn redact_token_like(text: &str) -> (String, usize) {
    let mut redacted = text.to_string();
    let mut count = 0usize;
    for (prefix, min_len) in TOKEN_PREFIXES {
        let mut search_from = 0usize;
        while let Some(relative) = redacted[search_from..].find(prefix) {
            let start = search_from + relative;
            let end = redacted[start..]
                .char_indices()
                .map(|(offset, _)| start + offset)
                .find(|&position| {
                    let is_token_byte = {
                        let byte = redacted.as_bytes()[position];
                        byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
                    };
                    !is_token_byte
                })
                .unwrap_or(redacted.len());
            if end - start >= *min_len {
                redacted.replace_range(
                    start..end,
                    &format!("[REDACTED:{}-token]", prefix.trim_end_matches(['-', '_'])),
                );
                count += 1;
                search_from = start + 1;
            } else {
                search_from = end.max(start + 1);
            }
        }
    }
    (redacted, count)
}

/// Convenience wrapper the executor's event recorder uses: values first,
/// then the generic token pass.
pub fn redact(text: &str, secret_values: &[String]) -> (String, usize) {
    let (text, value_hits) = redact_values(text, secret_values);
    let (text, token_hits) = redact_token_like(&text);
    (text, value_hits + token_hits)
}

/// Redact every string value in a JSON payload while preserving its shape.
///
/// Provider events and diagnostic records are persisted as structured JSON,
/// so redacting at this boundary prevents token-shaped values from reaching
/// durable storage without flattening the payload into an opaque string.
pub fn redact_json_value(value: &mut serde_json::Value, secret_values: &[String]) -> usize {
    match value {
        serde_json::Value::String(text) => {
            let (redacted, count) = redact(text, secret_values);
            *text = redacted;
            count
        }
        serde_json::Value::Array(values) => values
            .iter_mut()
            .map(|value| redact_json_value(value, secret_values))
            .sum(),
        serde_json::Value::Object(values) => values
            .values_mut()
            .map(|value| redact_json_value(value, secret_values))
            .sum(),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => 0,
    }
}

#[cfg(test)]
mod json_redaction_tests {
    use super::*;

    #[test]
    fn structured_json_is_redacted_without_losing_shape() {
        let mut value = serde_json::json!({
            "message": "before secret-value after",
            "nested": ["unchanged", 7, true]
        });

        let count = redact_json_value(&mut value, &["secret-value".to_string()]);

        assert_eq!(count, 1);
        assert!(!value["message"].as_str().unwrap().contains("secret-value"));
        assert_eq!(value["nested"][0], "unchanged");
        assert_eq!(value["nested"][1], 7);
        assert_eq!(value["nested"][2], true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_values_are_replaced_everywhere_and_counted() {
        let (text, count) = redact_values(
            "token=abc123 and again abc123 at the end",
            &["abc123".to_string()],
        );
        assert_eq!(count, 2);
        assert_eq!(text, "token=[REDACTED] and again [REDACTED] at the end");
        // An empty configured value would match everywhere; it is skipped.
        let (text, count) = redact_values("untouched", &[String::new()]);
        assert_eq!(count, 0);
        assert_eq!(text, "untouched");
    }

    #[test]
    fn known_token_shapes_are_redacted_even_when_unconfigured() {
        let (text, count) = redact_token_like(
            "header: Authorization: Bearer ghp_0123456789abcdefghijklmnopqrstuvwxyz",
        );
        assert_eq!(count, 1);
        assert!(text.contains("[REDACTED:ghp-token]"));
        assert!(!text.contains("ghp_0123"));
        let (text, count) = redact_token_like("key: sk-proj-0123456789abcdef0123456789");
        assert_eq!(count, 1);
        assert!(!text.contains("sk-proj-0123"));
    }

    #[test]
    fn ordinary_words_are_never_false_positives() {
        let (text, count) = redact_token_like("the task and skUNK argument list");
        assert_eq!(count, 0);
        assert_eq!(text, "the task and skUNK argument list");
    }

    #[test]
    fn the_combined_pass_redacts_values_then_shapes() {
        let (text, count) = redact(
            "OPENAI_KEY=sk-0123456789abcdef0123456789abcdef; db pass: hunter2exact",
            &["hunter2exact".to_string()],
        );
        assert_eq!(count, 2);
        assert!(text.contains("[REDACTED:sk-token]"));
        assert!(text.contains("[REDACTED]"));
        assert!(!text.contains("hunter2exact"));
    }
}
