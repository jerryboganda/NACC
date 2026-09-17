//! CI failure classification (master plan S19): a failed run is *classified
//! with evidence* before any repair is considered -- "Blind reruns do not
//! count as repairs" (S18.3). The classifier is deliberately rule-based
//! over the failed job's name and log text: deterministic, auditable, and
//! wrong in ways a human can see and correct, exactly like the gates.

/// The failure classes the master plan's CI/CD flow distinguishes. `Unknown`
/// is a real answer: an unclassified failure needs a human, not a rerun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureClass {
    ProductDefect,
    StaleTest,
    Flakiness,
    Environment,
    Dependency,
    WorkflowConfig,
    SecretMissing,
    ExternalService,
    Timeout,
    ResourceExhaustion,
    Permissions,
    Unknown,
}

impl FailureClass {
    /// The classification labels a repair workflow's diagnosis node; it never
    /// triggers a rerun by itself (S19.1).
    pub fn suggests_repair_run(self) -> bool {
        !matches!(self, FailureClass::Unknown)
    }
}

/// One keyword rule: a lowercase needle and the class it indicates. Ordered
/// by specificity -- the first match wins, so "connection reset by peer"
/// (external service) beats a bare "error".
const RULES: &[(&str, FailureClass)] = &[
    ("secret", FailureClass::SecretMissing),
    ("credential", FailureClass::SecretMissing),
    ("401 unauthorized", FailureClass::Permissions),
    ("403 forbidden", FailureClass::Permissions),
    ("access denied", FailureClass::Permissions),
    ("no space left", FailureClass::ResourceExhaustion),
    ("out of memory", FailureClass::ResourceExhaustion),
    ("oom", FailureClass::ResourceExhaustion),
    ("connection reset", FailureClass::ExternalService),
    ("connection refused", FailureClass::ExternalService),
    ("503 service unavailable", FailureClass::ExternalService),
    ("dns resolution", FailureClass::ExternalService),
    ("could not resolve host", FailureClass::ExternalService),
    ("no matching manifest", FailureClass::Dependency),
    ("failed to resolve", FailureClass::Dependency),
    ("version conflict", FailureClass::Dependency),
    ("lock file", FailureClass::Dependency),
    ("no such file or directory", FailureClass::Environment),
    ("is not recognized", FailureClass::Environment),
    ("command not found", FailureClass::Environment),
    ("on a windows runner", FailureClass::Environment),
    ("timeout", FailureClass::Timeout),
    ("timed out", FailureClass::Timeout),
    ("workflow file", FailureClass::WorkflowConfig),
    ("invalid workflow", FailureClass::WorkflowConfig),
    ("assertion", FailureClass::ProductDefect),
    ("test failed", FailureClass::ProductDefect),
    ("expect(", FailureClass::ProductDefect),
    ("panic", FailureClass::ProductDefect),
    ("flaky", FailureClass::Flakiness),
    ("passed on retry", FailureClass::Flakiness),
];

/// Classify one failed run from its job name plus failed-step log text.
/// Returns the class and the exact needle that matched -- the evidence line
/// a diagnosis must show before a repair is accepted (S19.1).
pub fn classify(job_name: &str, failed_log: &str) -> (FailureClass, Option<String>) {
    let haystack = format!("{job_name}\n{failed_log}").to_lowercase();
    for (needle, class) in RULES {
        if haystack.contains(needle) {
            return (*class, Some(needle.to_string()));
        }
    }
    (FailureClass::Unknown, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_defects_and_infrastructure_tell_themselves_apart() {
        let (class, evidence) = classify("test-suite", "assertion failed: left == right");
        assert_eq!(class, FailureClass::ProductDefect);
        assert_eq!(evidence.as_deref(), Some("assertion"));
        let (class, _) = classify("setup", "error: could not resolve host: crates.io");
        assert_eq!(class, FailureClass::ExternalService);
        let (class, _) = classify(
            "build",
            "error: failed to resolve dependencies for reqwest v0.4",
        );
        assert_eq!(class, FailureClass::Dependency);
        let (class, _) = classify("e2e", "Error: The operation was canceled due to timeout");
        assert_eq!(class, FailureClass::Timeout);
        let (class, _) = classify(
            "deploy",
            "deploy step: 403 forbidden to environment production",
        );
        assert_eq!(class, FailureClass::Permissions);
    }

    #[test]
    fn secrets_rank_above_generic_errors() {
        let (class, _) = classify(
            "integration",
            "error: secret NOT_FOUND for MY_TOKEN; then test failed",
        );
        assert_eq!(
            class,
            FailureClass::SecretMissing,
            "the first rule wins, so ordering is the policy"
        );
    }

    #[test]
    fn an_unclassifiable_failure_stays_unknown_and_never_auto_repairs() {
        let (class, evidence) = classify("weird", "exit code 137 with no other output");
        assert_eq!(class, FailureClass::Unknown);
        assert!(evidence.is_none());
        assert!(!class.suggests_repair_run());
    }
}
