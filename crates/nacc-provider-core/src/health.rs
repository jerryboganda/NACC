//! Provider health: the single, typed answer to "can this provider be used
//! right now, and if not, why?" (master plan S8.3's `health` field, S17.1's
//! prerequisite detection, S17.4's provider/account page).
//!
//! Health is deliberately computed from observed facts
//! ([`crate::InstallationProbe`] + [`crate::AuthProbe`]) rather than stored
//! independently -- two sources of truth about readiness is exactly how a
//! GUI ends up showing "Ready" for a provider that cannot launch.
//!
//! The distinction that matters most here is the one Phase 0 found live:
//! Copilot's ACP mode rejects a *valid* classic PAT with "not supported in
//! this mode", and Gemini CLI rejects a *valid but ineligible* account tier.
//! Neither is "no credential", so neither may be reported as
//! `Unauthenticated` -- a user told to "log in again" when they are already
//! logged in will keep failing, and NACC will have lied to them.

use serde::{Deserialize, Serialize};

use crate::capability::{AuthProbe, InstallationProbe};

/// Whether a provider+account+runtime combination is usable, and if not,
/// which one of the distinct blocking reasons applies. Closed vocabulary:
/// each variant maps to a different, actionable piece of Setup Wizard
/// guidance.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProviderHealth {
    /// Installed, authenticated, and usable in the probed configuration.
    Ready,
    /// The CLI was not found on this runtime. The Setup Wizard's guidance
    /// differs between native Windows and WSL2 (S17.1), which is why
    /// `RuntimeProfile` is part of the probe in the first place.
    NotInstalled,
    /// Installed, but there is no usable credential at all.
    Unauthenticated,
    /// Installed with a credential, but that credential does not authorize
    /// the requested mode/account tier. Never conflated with
    /// `Unauthenticated`.
    IneligibleCredential { detail: String },
    /// The CLI is present but its version is outside what this adapter
    /// supports (or it reports an output contract the adapter cannot
    /// parse).
    IncompatibleVersion { detail: String },
}

impl ProviderHealth {
    /// Derive health from the two probes a capability snapshot carries.
    /// `ineligible_detail` is `Some` when the probe reported the
    /// credential-present-but-wrong-mode case, which is deliberately its
    /// own input rather than something inferred from `AuthProbe`'s
    /// `authenticated: false` -- those are different failures.
    pub fn from_probes(
        installation: &InstallationProbe,
        auth: &AuthProbe,
        ineligible_detail: Option<String>,
    ) -> Self {
        if !installation.installed {
            return ProviderHealth::NotInstalled;
        }
        if let Some(detail) = ineligible_detail {
            return ProviderHealth::IneligibleCredential { detail };
        }
        if !auth.authenticated {
            return ProviderHealth::Unauthenticated;
        }
        ProviderHealth::Ready
    }

    /// Whether a run may be started. The only affirmative answer is
    /// `Ready` -- a caller must never treat an unknown state as runnable
    /// (master plan S2.7: never silently downgrade).
    pub fn is_runnable(&self) -> bool {
        matches!(self, ProviderHealth::Ready)
    }

    /// Short, user-facing guidance. The GUI displays this verbatim; it is
    /// never a substitute for `detail` on the variants that carry one.
    pub fn guidance(&self) -> &'static str {
        match self {
            ProviderHealth::Ready => "Ready",
            ProviderHealth::NotInstalled => {
                "The provider CLI is not installed on this runtime. Install it and re-detect."
            }
            ProviderHealth::Unauthenticated => {
                "The provider CLI is installed but not authenticated. Sign in with the provider's own mechanism."
            }
            ProviderHealth::IneligibleCredential { .. } => {
                "A credential is present but is not valid for this mode. Use a credential the provider accepts for it."
            }
            ProviderHealth::IncompatibleVersion { .. } => {
                "The installed provider CLI version is not supported by this adapter."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(ok: bool) -> InstallationProbe {
        InstallationProbe {
            installed: ok,
            executable_path: ok.then(|| "C:\\fake\\cli.exe".to_string()),
            version: ok.then(|| "1.2.3".to_string()),
        }
    }

    fn auth(ok: bool) -> AuthProbe {
        AuthProbe {
            authenticated: ok,
            account_label: ok.then(|| "user@example.invalid".to_string()),
            detail: None,
        }
    }

    #[test]
    fn missing_installation_outranks_every_other_concern() {
        // Even with a credential problem reported, "not installed" is the
        // actionable fact -- telling a user to re-login to a CLI they do
        // not have would be actively misleading.
        let health = ProviderHealth::from_probes(
            &installed(false),
            &auth(false),
            Some("credential rejected".into()),
        );
        assert_eq!(health, ProviderHealth::NotInstalled);
    }

    #[test]
    fn an_ineligible_credential_is_not_reported_as_unauthenticated() {
        let health = ProviderHealth::from_probes(
            &installed(true),
            &auth(false),
            Some("classic PAT not supported in ACP mode".into()),
        );
        assert_eq!(
            health,
            ProviderHealth::IneligibleCredential {
                detail: "classic PAT not supported in ACP mode".into()
            }
        );
        assert!(!health.is_runnable());
        assert!(health.guidance().contains("not valid for this mode"));
    }

    #[test]
    fn installed_and_authenticated_is_the_only_runnable_state() {
        let ready = ProviderHealth::from_probes(&installed(true), &auth(true), None);
        assert_eq!(ready, ProviderHealth::Ready);
        assert!(ready.is_runnable());

        for health in [
            ProviderHealth::NotInstalled,
            ProviderHealth::Unauthenticated,
            ProviderHealth::IneligibleCredential {
                detail: "x".into(),
            },
            ProviderHealth::IncompatibleVersion { detail: "x".into() },
        ] {
            assert!(
                !health.is_runnable(),
                "{health:?} must never be treated as runnable"
            );
            assert!(!health.guidance().is_empty());
        }
    }

    #[test]
    fn health_roundtrips_through_json_with_its_state_tag() {
        let json = serde_json::to_string(&ProviderHealth::Unauthenticated).unwrap();
        assert_eq!(json, r#"{"state":"unauthenticated"}"#);
        let back: ProviderHealth = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ProviderHealth::Unauthenticated);
    }
}
