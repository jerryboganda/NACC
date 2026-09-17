//! Structured handoff enforcement (master plan S9.3): Antigravity's output
//! may be less machine-readable than Claude's or Codex's streams, so the
//! plan requires the worker to write a JSON handoff to a NACC-provided
//! path, NACC to validate it against this schema, and the run to
//! cross-check the handoff's claims against Git and command evidence.
//! This module is the schema and the cross-check contract; the executor
//! consumes it once a real launch path exists.

use serde::Deserialize;

/// One file the handoff claims was created or modified.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ClaimedFile {
    pub path: String,
    pub change: ClaimedChange,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaimedChange {
    Created,
    Modified,
    Deleted,
}

/// The handoff document itself. Kept deliberately small: everything in it is
/// something NACC can independently verify, so nothing in it is taken on
/// trust.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct HandoffArtifact {
    /// The agent's own summary of what it did (shown to the user, never
    /// treated as evidence).
    pub summary: String,
    pub files: Vec<ClaimedFile>,
    /// Test commands the handoff claims were run and passed.
    pub tests_claimed_passed: Vec<String>,
}

impl HandoffArtifact {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Cross-check the handoff's file claims against the actual `git
    /// status --porcelain` output of the worktree. Returns the claims that
    /// do NOT hold: an empty result means every claimed change is visible
    /// in Git.
    pub fn unverified_file_claims(&self, porcelain: &str) -> Vec<String> {
        self.files
            .iter()
            .filter(|claim| {
                // Porcelain lines look like "M  src/x.rs" or "?? new.txt";
                // the path is everything after the two status columns.
                !porcelain
                    .lines()
                    .any(|line| line.len() > 3 && line[3..].trim_matches('"') == claim.path)
            })
            .map(|claim| claim.path.clone())
            .collect()
    }

    /// Cross-check the handoff's test claims against the quality gate
    /// evidence actually recorded (S27.19: deterministic gates, not model
    /// claims, determine success). A claimed test with no matching passing
    /// gate is unverified.
    pub fn unverified_test_claims(&self, passed_gates: &[String]) -> Vec<String> {
        self.tests_claimed_passed
            .iter()
            .filter(|claimed| !passed_gates.iter().any(|gate| gate == *claimed))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "summary": "fixed the parser",
        "files": [
            {"path": "src/parser.rs", "change": "modified"},
            {"path": "src/new_module.rs", "change": "created"}
        ],
        "tests_claimed_passed": ["cargo test -p parser"]
    }"#;

    #[test]
    fn a_well_formed_handoff_parses_whole() {
        let handoff = HandoffArtifact::from_json(SAMPLE).unwrap();
        assert_eq!(handoff.files.len(), 2);
        assert_eq!(handoff.files[0].change, ClaimedChange::Modified);
        assert_eq!(handoff.tests_claimed_passed.len(), 1);
    }

    #[test]
    fn a_handoff_missing_required_fields_is_refused() {
        assert!(HandoffArtifact::from_json("{\"summary\":\"x\"}").is_err());
        assert!(HandoffArtifact::from_json("not json at all").is_err());
    }

    #[test]
    fn file_claims_are_cross_checked_against_git_porcelain_not_trusted() {
        let handoff = HandoffArtifact::from_json(SAMPLE).unwrap();
        let porcelain = " M src/parser.rs\n?? src/new_module.rs\n";
        assert!(
            handoff.unverified_file_claims(porcelain).is_empty(),
            "every claim visible in git counts as verified"
        );
        let porcelain_missing_one = " M src/parser.rs\n";
        assert_eq!(
            handoff.unverified_file_claims(porcelain_missing_one),
            vec!["src/new_module.rs".to_string()],
            "a claimed file git does not show is flagged, not believed"
        );
    }

    #[test]
    fn test_claims_are_cross_checked_against_gate_evidence_not_trusted() {
        let handoff = HandoffArtifact::from_json(SAMPLE).unwrap();
        assert!(handoff
            .unverified_test_claims(&["cargo test -p parser".to_string()])
            .is_empty());
        assert_eq!(
            handoff.unverified_test_claims(&[]),
            vec!["cargo test -p parser".to_string()]
        );
    }
}
