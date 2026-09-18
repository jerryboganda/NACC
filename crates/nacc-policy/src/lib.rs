//! The policy engine (master plan S12, S13, S27.34): typed decisions about
//! paths, commands, and the danger-mode override, evaluated before a
//! privileged operation happens. Rules are data, decisions carry reasons,
//! and "allow" is never assumed silently -- every check returns a
//! [`Decision`] the caller must be able to show the user.

use std::path::{Path, PathBuf};

/// One explicit grant of the TemporaryDangerFullAccess profile (S12.1):
/// per-run, time-limited, with a typed scope note. It cannot extend or
/// renew itself -- a new grant is a new, separately audited decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DangerGrant {
    pub scope_note: String,
    pub expires_at_millis: u64,
}

impl DangerGrant {
    pub fn is_valid_at(&self, now_millis: u64) -> bool {
        now_millis < self.expires_at_millis
    }
}

/// What the engine configures. Everything the plan names as a protected
/// rule class is represented: protected paths (no writes, ever), denied
/// command fragments (matched as exact argument substrings), and the
/// optional, expiring danger grant.
#[derive(Clone, Debug, Default)]
pub struct PolicyEngine {
    protected_paths: Vec<PathBuf>,
    denied_command_fragments: Vec<String>,
    danger_grant: Option<DangerGrant>,
}

/// The outcome of every check. `Allow` carries the rule that permitted it;
/// `Deny` carries the reason a human will see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow { rule: &'static str },
    Deny { reason: String },
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Baseline guardrails that are always required for NACC-owned
    /// privileged command execution. Resource-specific protected paths and
    /// temporary grants are layered on by the caller.
    pub fn baseline() -> Self {
        Self::new()
            .deny_command_fragment("push --force")
            .deny_command_fragment("branch -D main")
    }

    pub fn protect_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.protected_paths.push(path.into());
        self
    }

    pub fn deny_command_fragment(mut self, fragment: impl Into<String>) -> Self {
        self.denied_command_fragments.push(fragment.into());
        self
    }

    pub fn grant_danger_mode(&mut self, grant: DangerGrant) {
        self.danger_grant = Some(grant);
    }

    /// Is the danger-mode override currently valid? Expired grants are
    /// reported as `None` -- expiry is enforced at read time, so a grant
    /// cannot outlive itself by being checked late.
    pub fn danger_mode_at(&self, now_millis: u64) -> Option<&DangerGrant> {
        self.danger_grant
            .as_ref()
            .filter(|grant| grant.is_valid_at(now_millis))
    }

    /// May this process write to `path`? Protected paths refuse absolutely;
    /// a currently-valid danger grant does NOT punch through them (S12.2's
    /// always-gated list is not purchasable with a danger override).
    pub fn check_write(&self, path: &Path, now_millis: u64) -> Decision {
        for protected in &self.protected_paths {
            if path.starts_with(protected) {
                return Decision::Deny {
                    reason: format!(
                        "{} is a protected path; writes are refused regardless of danger mode",
                        protected.display()
                    ),
                };
            }
        }
        if self.danger_mode_at(now_millis).is_some() {
            Decision::Allow {
                rule: "danger grant valid at check time",
            }
        } else {
            Decision::Allow {
                rule: "path not protected (permission profile still applies at the executor)",
            }
        }
    }

    /// May this command line run? Denied fragments are exact substrings of
    /// the joined argument list -- matching is case-insensitive because the
    /// things being denied (force-pushes, branch deletions) are typed by
    /// agents in any case they like.
    pub fn check_command(&self, argv: &[String], now_millis: u64) -> Decision {
        let joined = argv.join(" ").to_lowercase();
        for fragment in &self.denied_command_fragments {
            if joined.contains(&fragment.to_lowercase()) {
                let danger_active = self.danger_mode_at(now_millis).is_some();
                return Decision::Deny {
                    reason: format!(
                        "command matches denied fragment {fragment:?}{}",
                        if danger_active {
                            " (denied even under danger mode)"
                        } else {
                            ""
                        }
                    ),
                };
            }
        }
        Decision::Allow {
            rule: "no denied fragment matched",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> PolicyEngine {
        PolicyEngine::baseline()
            .protect_path(r"C:\Users\me\.ssh")
            .protect_path(r"D:\Projects\NACC")
    }

    #[test]
    fn protected_paths_refuse_writes_even_under_danger_mode() {
        let mut policy = engine();
        policy.grant_danger_mode(DangerGrant {
            scope_note: "one migration attempt".to_string(),
            expires_at_millis: 2_000,
        });
        let decision = policy.check_write(Path::new(r"C:\Users\me\.ssh\id_ed25519"), 1_000);
        assert!(matches!(decision, Decision::Deny { .. }));
        let allowed = policy.check_write(Path::new(r"D:\other\repo"), 1_000);
        assert!(matches!(allowed, Decision::Allow { .. }));
    }

    #[test]
    fn danger_grants_expire_at_read_time() {
        let mut policy = engine();
        policy.grant_danger_mode(DangerGrant {
            scope_note: "expired already".to_string(),
            expires_at_millis: 1_000,
        });
        assert!(policy.danger_mode_at(999).is_some());
        assert!(
            policy.danger_mode_at(1_000).is_none(),
            "expiry is not >= the expiry instant"
        );
    }

    #[test]
    fn denied_command_fragments_match_in_any_case_with_reasons() {
        let policy = engine();
        let argv = vec![
            "git".to_string(),
            "push".to_string(),
            "--FORCE".to_string(),
            "origin".to_string(),
        ];
        let decision = policy.check_command(&argv, 0);
        match decision {
            Decision::Deny { reason } => assert!(reason.contains("push --force")),
            Decision::Allow { .. } => panic!("a force-push must be denied with a reason"),
        }
        let allowed = policy.check_command(&["git".to_string(), "status".to_string()], 0);
        assert!(matches!(allowed, Decision::Allow { .. }));
    }
}
