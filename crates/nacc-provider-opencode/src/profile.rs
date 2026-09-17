//! Gateway account profiles (master plan S9.5): the configuration shape the
//! account editor presents for OpenCode-backed gateways -- TokenRouter, B.AI,
//! DeepSeek/GLM/Qwen-family endpoints, and any OpenAI- or Anthropic-compatible
//! base URL. Every field the plan lists exists here, and nothing is assumed:
//! the exact models a gateway returns are displayed as returned, and the
//! user may add model IDs by hand when discovery is unavailable.

use serde::{Deserialize, Serialize};

/// How NACC talks to the gateway. The plan names OpenAI-compatible and
/// Anthropic-compatible protocols; anything else is "custom" and needs the
/// user's explicit description rather than a guessed default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayProtocol {
    OpenAiCompatible,
    AnthropicCompatible,
    Custom,
}

/// How NACC should read a rate-limit rejection from this gateway.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitInterpretation {
    /// 429 means "retry after the reported delay".
    RetryAfterDelay,
    /// 429 means "this credential is exhausted for now" -- fail over to the
    /// profile's fallback instead of waiting.
    FailOver,
    /// The gateway's semantics are unknown; treat 429 as retryable without
    /// inventing a delay.
    Unverified,
}

/// One gateway profile: everything S9.5 requires the account editor to
/// support, no more and no less. `credential_reference` is the *name* of a
/// credential in the user's own store -- never the credential itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayProfile {
    pub label: String,
    pub base_url: String,
    pub protocol: GatewayProtocol,
    /// Name of the credential in the user's credential store. NACC stores
    /// the reference, not the secret (S13.5).
    pub credential_reference: Option<String>,
    /// Extra headers sent with every request, as (name, value) pairs. Values
    /// the user typed are sent verbatim and redacted from any log.
    pub custom_headers: Vec<(String, String)>,
    /// Manually added model IDs for when discovery is unavailable.
    pub manual_model_ids: Vec<String>,
    /// Maximum context the user says this gateway/model supports, if known.
    pub max_context_tokens: Option<u64>,
    /// Request timeout in seconds.
    pub request_timeout_secs: u32,
    /// Concurrent requests NACC may have in flight to this gateway.
    pub concurrency_limit: u32,
    pub rate_limits: RateLimitInterpretation,
    /// Pricing the user entered when the gateway does not expose it, as
    /// (model_id, currency_usd_per_million_input_tokens) pairs.
    pub pricing_metadata: Vec<(String, f64)>,
}

impl Default for GatewayProfile {
    fn default() -> Self {
        Self {
            label: String::new(),
            base_url: String::new(),
            protocol: GatewayProtocol::OpenAiCompatible,
            credential_reference: None,
            custom_headers: Vec::new(),
            manual_model_ids: Vec::new(),
            max_context_tokens: None,
            request_timeout_secs: 300,
            concurrency_limit: 2,
            rate_limits: RateLimitInterpretation::Unverified,
            pricing_metadata: Vec::new(),
        }
    }
}

impl GatewayProfile {
    /// Validation the account editor runs before anything is saved: a
    /// profile that cannot possibly work is refused with the reason, not
    /// stored to fail later.
    pub fn validate(&self) -> Result<(), String> {
        if self.label.trim().is_empty() {
            return Err("gateway profile needs a label".to_string());
        }
        if !self.base_url.starts_with("https://") {
            // A gateway request carries the credential reference's secret;
            // plaintext HTTP is refused outright (master plan S13).
            return Err("gateway base_url must be https".to_string());
        }
        if self.protocol == GatewayProtocol::Custom
            && self.custom_headers.is_empty()
            && self.manual_model_ids.is_empty()
        {
            return Err("a custom protocol profile needs at least a header or a manual model id to be usable".to_string());
        }
        if self.request_timeout_secs == 0 || self.concurrency_limit == 0 {
            return Err("request timeout and concurrency limit must be at least 1".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_profile_validates() {
        let profile = GatewayProfile {
            label: "TokenRouter".to_string(),
            base_url: "https://gw.example/v1".to_string(),
            manual_model_ids: vec!["glm-family-large".to_string()],
            ..GatewayProfile::default()
        };
        assert_eq!(profile.validate(), Ok(()));
    }

    #[test]
    fn plaintext_http_and_empty_labels_are_refused() {
        let mut profile = GatewayProfile::default();
        assert!(profile.validate().is_err(), "empty label refused");
        profile.label = "B.AI".to_string();
        profile.base_url = "http://gw.example/v1".to_string();
        assert!(profile.validate().is_err(), "plaintext http refused");
        profile.base_url = "https://gw.example/v1".to_string();
        profile.protocol = GatewayProtocol::Custom;
        assert!(
            profile.validate().is_err(),
            "custom protocol with nothing usable refused"
        );
    }

    #[test]
    fn the_profile_round_trips_through_json_with_defaults() {
        let profile = GatewayProfile {
            label: "DeepSeek relay".to_string(),
            base_url: "https://api.deepseek.example/v1".to_string(),
            ..GatewayProfile::default()
        };
        let json = serde_json::to_string(&profile).unwrap();
        let back: GatewayProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, profile);
        // A stored profile from an older build without newer fields still
        // reads, via `#[serde(default)]`.
        let old: GatewayProfile =
            serde_json::from_str(r#"{"label":"x","base_url":"https://a.b/c"}"#).unwrap();
        assert_eq!(old.rate_limits, RateLimitInterpretation::Unverified);
    }
}
