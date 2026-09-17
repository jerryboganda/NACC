//! Independent review findings (master plan S17.10, S2.6, S27.20-22): the
//! typed record a reviewer role produces per finding, so review output is
//! structured evidence rather than prose a human must re-read. Review is
//! assigned to a *different provider family* than implementation by the
//! templates and the cross-provider rule in the app layer; this crate owns
//! the finding shape and its rules.

/// How a finding affects the run. `Blocker` and `Major` findings must be
/// repaired (bounded, S14.5) or explicitly waived by a human before
/// integration; `Minor` and `Note` never block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Blocker,
    Major,
    Minor,
    Note,
}

impl Severity {
    pub fn blocks_integration(&self) -> bool {
        matches!(self, Severity::Blocker | Severity::Major)
    }
}

/// One structured finding. `file` + `line` point at the evidence; `summary`
/// is the one-sentence claim; `evidence` is the concrete failure scenario
/// ("file:line plus a concrete failure scenario", template contract). A
/// finding without evidence is not a finding.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewFinding {
    pub node_key: String,
    pub file: String,
    pub line: Option<u32>,
    pub severity: Severity,
    pub summary: String,
    pub evidence: String,
}

impl ReviewFinding {
    pub fn validate(&self) -> Result<(), String> {
        if self.node_key.trim().is_empty() {
            return Err("a finding must name the reviewing node".to_string());
        }
        if self.file.trim().is_empty() {
            return Err("a finding must point at a file".to_string());
        }
        if self.summary.trim().is_empty() {
            return Err("a finding must state its claim".to_string());
        }
        if self.evidence.trim().is_empty() {
            return Err("a finding must carry concrete evidence -- a claim without a failure scenario is an opinion".to_string());
        }
        Ok(())
    }
}

/// Which findings, if any, block integration of the reviewed work.
pub fn blocking(findings: &[ReviewFinding]) -> Vec<&ReviewFinding> {
    findings
        .iter()
        .filter(|f| f.severity.blocks_integration())
        .collect()
}

/// The bounded repair rule (master plan S14.5): no more than two automated
/// repair cycles for the same failure signature. `repair_allowed` answers
/// it from the count of *prior repair attempts at this same signature*.
pub fn repair_allowed(prior_repair_attempts_for_signature: u32) -> bool {
    prior_repair_attempts_for_signature < 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(severity: Severity) -> ReviewFinding {
        ReviewFinding {
            node_key: "review".to_string(),
            file: "src/parser.rs".to_string(),
            line: Some(42),
            severity,
            summary: "unbounded loop on adversarial input".to_string(),
            evidence: "input `[[[[...` of length 1e6 loops without progress; see fuzz case 7"
                .to_string(),
        }
    }

    #[test]
    fn a_finding_without_evidence_is_not_a_finding() {
        let mut empty = finding(Severity::Major);
        empty.evidence = "  ".to_string();
        assert!(ReviewFinding::validate(&empty).is_err());
        assert_eq!(ReviewFinding::validate(&finding(Severity::Note)), Ok(()));
    }

    #[test]
    fn only_blocker_and_major_findings_block_integration() {
        let findings = vec![
            finding(Severity::Blocker),
            finding(Severity::Major),
            finding(Severity::Minor),
            finding(Severity::Note),
        ];
        assert_eq!(blocking(&findings).len(), 2);
    }

    #[test]
    fn repair_loops_are_bounded_at_two_attempts() {
        assert!(repair_allowed(0));
        assert!(repair_allowed(1));
        assert!(
            !repair_allowed(2),
            "S14.5: no more than two automated repair cycles"
        );
    }
}
