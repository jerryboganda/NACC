//! Release and quarantine: how a worktree lease ends (master plan S16 rule
//! 9, S17.9).
//!
//! The rule this file exists to implement is stated in the master plan as
//! "a dirty worktree is quarantined rather than destroyed", and it is
//! exactly as strict as it sounds: the only automatic deletion path here is
//! a worktree with a clean working tree *and* zero commits beyond its
//! recorded base. Anything else is moved aside, intact, with a machine-
//! readable manifest describing it.
//!
//! Quarantine moves the directory rather than deleting any part of it --
//! uncommitted files included -- and then asks Git to prune its (now stale)
//! registration. The quarantined directory is a complete, ordinary working
//! tree: `cd` into it, or point any Git client at it, and the work is all
//! there. That is the difference between quarantine and a `--force`
//! removal, and it is the whole point.

use std::path::PathBuf;

use nacc_domain::{WorktreeLease, WorktreeState};

use crate::{now_millis, LeaseInspection, Result, WorktreeDrift, WorktreeError, WorktreeManager};

/// What the caller wants to happen to a worktree when its run ends.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum ReleasePolicy {
    /// Remove it if it is safe to do so, quarantine it otherwise. This is
    /// the default for automated cleanup.
    RemoveIfSafe,
    /// Touch nothing. Used when a human has explicitly asked to keep the
    /// worktree, or when the lease is being handed off.
    Keep,
}

/// What actually happened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseOutcome {
    Removed { path: PathBuf, branch: String },
    /// Moved aside with everything intact. `destination` is where the work
    /// now lives; `reason` says why it was not safe to remove.
    Quarantined {
        destination: PathBuf,
        reason: String,
    },
    /// The directory was already gone -- bookkeeping only, nothing was
    /// deleted (and nothing could have been).
    AlreadyAbsent { path: PathBuf },
    Kept { reason: String },
}

/// A release plus the evidence it was based on.
#[derive(Clone, Debug)]
pub struct ReleaseReport {
    pub lease: WorktreeLease,
    pub inspection: LeaseInspection,
    pub outcome: ReleaseOutcome,
}

impl WorktreeManager {
    /// End a lease under `policy`. Always leaves the lease record in a
    /// terminal state (`Released` or `Quarantined`) on success -- a lease
    /// that is neither is exactly the state reconciliation exists to
    /// resolve.
    pub async fn release(
        &self,
        repo: &nacc_git::GitRepository,
        lease: &WorktreeLease,
        policy: ReleasePolicy,
    ) -> Result<ReleaseReport> {
        let inspection = self.inspect(repo, lease).await?;

        if matches!(policy, ReleasePolicy::Keep) {
            return Ok(ReleaseReport {
                lease: lease.clone(),
                inspection,
                outcome: ReleaseOutcome::Kept {
                    reason: "release policy is Keep; nothing was changed".to_string(),
                },
            });
        }

        if !inspection.path_exists {
            let updated = self.mark_released(lease, "worktree directory was already absent").await?;
            return Ok(ReleaseReport {
                lease: updated,
                inspection: inspection.clone(),
                outcome: ReleaseOutcome::AlreadyAbsent {
                    path: PathBuf::from(&lease.path),
                },
            });
        }

        if inspection.must_preserve() {
            let reason = preserve_reason(&inspection.drift);
            let (updated, destination) = self.quarantine_lease(repo, lease, &reason).await?;
            return Ok(ReleaseReport {
                lease: updated,
                inspection,
                outcome: ReleaseOutcome::Quarantined { destination, reason },
            });
        }

        let path = PathBuf::from(&lease.path);
        repo.remove_worktree(&path, false).await?;
        let updated = self.mark_released(lease, "clean worktree removed").await?;
        tracing::info!(
            lease_id = %updated.id,
            path = %path.display(),
            branch = %lease.branch,
            "released clean worktree"
        );
        Ok(ReleaseReport {
            lease: updated,
            inspection,
            outcome: ReleaseOutcome::Removed {
                path,
                branch: lease.branch.clone(),
            },
        })
    }

    /// Quarantine a lease explicitly (e.g. a human clicking "preserve this
    /// worktree", or reconciliation finding a crashed run's dirty tree).
    /// Returns the updated lease and the destination directory.
    pub async fn quarantine_lease(
        &self,
        repo: &nacc_git::GitRepository,
        lease: &WorktreeLease,
        reason: &str,
    ) -> Result<(WorktreeLease, PathBuf)> {
        let origin = PathBuf::from(&lease.path);
        if !origin.exists() {
            let updated = self
                .mark_released(lease, "nothing to quarantine: worktree directory is absent")
                .await?;
            return Ok((updated, origin));
        }

        let quarantine_root = origin
            .parent()
            .map(|parent| parent.join(".quarantine"))
            .ok_or_else(|| {
                WorktreeError::Other(format!(
                    "worktree path {origin:?} has no parent directory to quarantine into"
                ))
            })?;
        let file_name = origin
            .file_name()
            .ok_or_else(|| WorktreeError::Other(format!("worktree path {origin:?} has no name")))?
            .to_owned();
        let destination = quarantine_root.join(file_name);

        // Never overwrite an existing quarantine slot: that would destroy
        // previously preserved work, which is the one thing quarantine is
        // supposed to make impossible.
        if destination.exists() {
            return Err(WorktreeError::QuarantineTargetExists {
                path: origin,
                destination,
            });
        }
        std::fs::create_dir_all(&quarantine_root)?;
        std::fs::rename(&origin, &destination)?;

        // The manifest is deliberately adjacent to the preserved files
        // rather than only in the database: whoever finds this directory
        // (a human, a future restore command) can tell what it is without
        // NACC's database at all.
        let manifest = serde_json::json!({
            "nacc_quarantine": {
                "lease_id": lease.id.to_string(),
                "project_id": lease.project_id.to_string(),
                "workflow_run_id": lease.workflow_run_id.map(|id| id.to_string()),
                "node_run_id": lease.node_run_id.map(|id| id.to_string()),
                "branch": lease.branch,
                "base_commit": lease.base_commit,
                "head_commit_at_quarantine": lease.head_commit,
                "original_path": lease.path,
                "reason": reason,
                "quarantined_at_millis": now_millis(),
            }
        });
        std::fs::write(
            destination.join("NACC-QUARANTINE.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;

        // Git still believes the worktree lives at the old path. Pruning
        // only drops that stale registration; it does not touch the moved
        // files. Best-effort and logged rather than fatal: the work is
        // already preserved at this point, and a stale `git worktree list`
        // entry is cosmetic compared to rolling back a completed
        // quarantine.
        if let Err(err) = repo.prune_worktrees().await {
            tracing::warn!(
                lease_id = %lease.id,
                error = %err,
                "quarantined the worktree but failed to prune git's stale registration for it"
            );
        }

        let mut updated = lease.clone();
        updated.path = destination.to_string_lossy().into_owned();
        updated.state = WorktreeState::Quarantined;
        updated.quarantine_reason = Some(format!("{reason} (moved from {})", lease.path));
        updated.updated_at_millis = now_millis();
        self.storage().update_worktree_lease(&updated).await?;

        tracing::warn!(
            lease_id = %updated.id,
            destination = %destination.display(),
            reason = %reason,
            "quarantined worktree (preserved, not deleted)"
        );
        Ok((updated, destination))
    }

    pub(crate) async fn mark_released(
        &self,
        lease: &WorktreeLease,
        reason: &str,
    ) -> Result<WorktreeLease> {
        let mut updated = lease.clone();
        updated.state = WorktreeState::Released;
        updated.quarantine_reason = None;
        updated.updated_at_millis = now_millis();
        self.storage().update_worktree_lease(&updated).await?;
        tracing::debug!(lease_id = %updated.id, reason, "lease released");
        Ok(updated)
    }

}

/// Human-readable reason string for a preservation decision, built from the
/// actual drift facts rather than a generic "dirty".
fn preserve_reason(drift: &[WorktreeDrift]) -> String {
    let mut reasons = Vec::new();
    for item in drift {
        match item {
            WorktreeDrift::DirtyWorkingTree => {
                reasons.push("uncommitted changes in the working tree".to_string())
            }
            WorktreeDrift::UnintegratedCommits { count } => reasons.push(format!(
                "{count} commit(s) not reachable from the lease base commit"
            )),
            WorktreeDrift::MissingPath => {
                reasons.push("working tree directory is missing".to_string())
            }
            WorktreeDrift::NotRegistered => {
                reasons.push("no longer registered with git".to_string())
            }
            WorktreeDrift::BranchChanged { expected, actual } => {
                reasons.push(format!("branch changed from {expected} to {actual}"))
            }
            WorktreeDrift::HeadMoved { recorded, current } => {
                reasons.push(format!("HEAD moved from {recorded} to {current}"))
            }
            WorktreeDrift::DetachedHead => reasons.push("detached HEAD".to_string()),
        }
    }
    if reasons.is_empty() {
        "preservation requested".to_string()
    } else {
        format!("preserved because: {}", reasons.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::{WorktreeManager, WorktreeState};
    use nacc_domain::ProjectId;
    use nacc_storage::Database;

    #[tokio::test]
    async fn a_clean_worktree_at_its_base_is_removed() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db.clone());
        let lease = manager
            .allocate(&repo, request(ProjectId::new(), &root, "Clean"))
            .await
            .unwrap();
        let path = PathBuf::from(&lease.path);

        let report = manager
            .release(&repo, &lease, ReleasePolicy::RemoveIfSafe)
            .await
            .unwrap();

        assert!(matches!(report.outcome, ReleaseOutcome::Removed { .. }));
        assert!(!path.exists(), "a clean, unintegrated-nothing worktree is removed");
        assert_eq!(
            db.get_worktree_lease(lease.id)
                .await
                .unwrap()
                .unwrap()
                .state,
            WorktreeState::Released
        );
    }

    #[tokio::test]
    async fn a_dirty_worktree_is_quarantined_with_its_files_and_a_manifest() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db.clone());
        let lease = manager
            .allocate(&repo, request(ProjectId::new(), &root, "Dirty"))
            .await
            .unwrap();
        let worktree = PathBuf::from(&lease.path);
        std::fs::write(worktree.join("unfinished.rs"), "fn todo() {}\n").unwrap();

        let report = manager
            .release(&repo, &lease, ReleasePolicy::RemoveIfSafe)
            .await
            .unwrap();

        let destination = match &report.outcome {
            ReleaseOutcome::Quarantined { destination, reason } => {
                assert!(
                    reason.contains("uncommitted changes"),
                    "the reason must name the actual finding: {reason}"
                );
                destination.clone()
            }
            other => panic!("expected quarantine, got {other:?}"),
        };
        assert!(!worktree.exists(), "the worktree moved rather than staying put");
        assert_eq!(
            std::fs::read_to_string(destination.join("unfinished.rs")).unwrap(),
            "fn todo() {}\n",
            "the preserved file must be byte-identical"
        );
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(destination.join("NACC-QUARANTINE.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            manifest["nacc_quarantine"]["lease_id"],
            serde_json::Value::String(lease.id.to_string())
        );

        let stored = db.get_worktree_lease(lease.id).await.unwrap().unwrap();
        assert_eq!(stored.state, WorktreeState::Quarantined);
        assert!(stored.quarantine_reason.unwrap().contains("uncommitted changes"));
        assert_eq!(stored.path, destination.to_string_lossy());
    }

    #[tokio::test]
    async fn committed_but_unintegrated_work_is_also_preserved() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db);
        let lease = manager
            .allocate(&repo, request(ProjectId::new(), &root, "Committed"))
            .await
            .unwrap();
        let worktree = PathBuf::from(&lease.path);

        let worktree_repo = nacc_git::GitRepository::open(&worktree).await.unwrap();
        worktree_repo
            .configure_identity("a@b.invalid", "Agent")
            .await
            .unwrap();
        std::fs::write(worktree.join("feature.rs"), "pub fn feature() {}\n").unwrap();
        worktree_repo.stage_all().await.unwrap();
        worktree_repo.commit("agent work").await.unwrap();

        let report = manager
            .release(&repo, &lease, ReleasePolicy::RemoveIfSafe)
            .await
            .unwrap();
        match report.outcome {
            ReleaseOutcome::Quarantined { reason, .. } => assert!(
                reason.contains("not reachable from the lease base"),
                "the reason must name unintegrated commits: {reason}"
            ),
            other => panic!("committed work must never be dropped: {other:?}"),
        }
    }

    #[tokio::test]
    async fn keep_policy_changes_nothing_at_all() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db.clone());
        let lease = manager
            .allocate(&repo, request(ProjectId::new(), &root, "Keep"))
            .await
            .unwrap();

        let report = manager
            .release(&repo, &lease, ReleasePolicy::Keep)
            .await
            .unwrap();
        assert!(matches!(report.outcome, ReleaseOutcome::Kept { .. }));
        assert!(PathBuf::from(&lease.path).is_dir());
        assert_eq!(
            db.get_worktree_lease(lease.id).await.unwrap().unwrap().state,
            WorktreeState::Active,
            "a kept lease stays active so reconciliation still owns it"
        );
    }

    #[tokio::test]
    async fn releasing_an_already_absent_worktree_only_corrects_bookkeeping() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db.clone());
        let lease = manager
            .allocate(&repo, request(ProjectId::new(), &root, "Absent"))
            .await
            .unwrap();
        std::fs::remove_dir_all(&lease.path).unwrap();

        let report = manager
            .release(&repo, &lease, ReleasePolicy::RemoveIfSafe)
            .await
            .unwrap();
        assert!(matches!(report.outcome, ReleaseOutcome::AlreadyAbsent { .. }));
        assert_eq!(
            db.get_worktree_lease(lease.id).await.unwrap().unwrap().state,
            WorktreeState::Released
        );
    }

    #[test]
    fn preserve_reason_names_real_findings() {
        let reason = preserve_reason(&[
            WorktreeDrift::DirtyWorkingTree,
            WorktreeDrift::UnintegratedCommits { count: 3 },
        ]);
        assert!(reason.contains("uncommitted changes"));
        assert!(reason.contains("3 commit(s)"));
        assert_eq!(preserve_reason(&[]), "preservation requested");
    }
}
