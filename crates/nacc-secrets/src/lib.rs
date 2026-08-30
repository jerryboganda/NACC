//! Credential references and redaction; NACC-owned secrets go to Windows Credential Manager, never plaintext.
//!
//! Master plan S13.5 -- Phase 11 scope for real credential storage. This crate establishes the workspace boundary and
//! its typed error vocabulary in Phase 1; the logic listed above is
//! deliberately not implemented yet, matching the phased roadmap (build
//! prompt S17 / master plan S24) rather than front-loading work into a
//! phase that is not scoped to deliver it.

/// Placeholder error type for this crate. Real, specific variants are
/// added as the crate gains real logic in its target phase; a single
/// `Other` variant with a message is deliberately the only case for now
/// so downstream code that already matches on this type does not need to
/// change shape later, only grow more specific arms.
#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SecretsError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_displays_its_message() {
        let err = SecretsError::Other("boundary established".into());
        assert_eq!(err.to_string(), "boundary established");
    }
}
