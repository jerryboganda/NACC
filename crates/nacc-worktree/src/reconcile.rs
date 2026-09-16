//! Startup reconciliation (master plan S16, S17.15, and the completion
//! criterion "restart NACC during a controlled run and prove safe
//! recovery").
//!
//! After an unclean shutdown, NACC's database can say "lease L is active,
//! owned by process P, at path X" while the truth is any of: P is still
//! running; P died and X is a half-finished tree; P died and X is gone. This
//! module answers that question for every active lease and takes exactly one
//! safe action per case -- and, critically, it never deletes work that a
//! crashed run left behind.
//!
//! It is also deliberately split from *acting* on leases during a run: the
//! only caller is app startup and an explicit "reconcile now" action
//! (`src-tauri`'), so a running lease is never reconciled out from under a
//! live agent.

use std::path::{Path, PathBuf};

use nacc_domain::{ProjectId, WorktreeLease, WorktreeLeaseId};

use crate::{paths_equal, ReleaseOutcome, Result, WorktreeManager};

/// One thing reconciliation did (or deliberately did not do).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconciliationAction {
    /// The lease's recorded owner process is still alive: leave it alone.
    /// Reported rather than silently skipped, because "a run survived a
    /// NACC restart" is exactly the fact a user needs to see.
    OwnerStillRunning {
        lease_id: WorktreeLeaseId,
        owner_process_id: u32,
    },
    /// Clean worktree, dead owner: safely removed and the lease released.
    Released {
        lease_id: WorktreeLeaseId,
        path: PathBuf,
    },
    /// Dirty or unintegrated work, dead owner: moved aside intact.
    Quarantined {
        lease_id: WorktreeLeaseId,
        destination: PathBuf,
        reason: String,
    },
    /// The directory was already gone: bookkeeping corrected only.
    AlreadyAbsent {
        lease_id: WorktreeLeaseId,
        path: PathBuf,
    },
    /// A worktree exists under NACC's managed root with no lease row at
    /// all (created by an older build, deleted database, or a human).
    /// Never touched -- reported so the user can decide.
    UnmanagedWorktree { path: PathBuf },
}

/// The full result of one reconciliation pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconciliationReport {
    pub actions: Vec<ReconciliationAction>,
}

impl ReconciliationReport {
    pub fn unmanaged(&self) -> Vec<&PathBuf> {
        self.actions
            .iter()
            .filter_map(|action| match action {
                ReconciliationAction::UnmanagedWorktree { path } => Some(path),
                _ => None,
            })
            .collect()
    }

    pub fn quarantined(&self) -> usize {
        self.actions
            .iter()
            .filter(|action| matches!(action, ReconciliationAction::Quarantined { .. }))
            .count()
    }

    pub fn released(&self) -> usize {
        self.actions
            .iter()
            .filter(|action| matches!(action, ReconciliationAction::Released { .. }))
            .count()
    }

    /// Whether anything was found that a human should look at.
    pub fn needs_attention(&self) -> bool {
        self.actions.iter().any(|action| {
            matches!(
                action,
                ReconciliationAction::Quarantined { .. }
                    | ReconciliationAction::UnmanagedWorktree { .. }
            )
        })
    }
}

impl WorktreeManager {
    /// Reconcile every active lease for `repo`. `project_id` scopes the
    /// lease query; the unmanaged-worktree scan is scoped by
    /// `worktrees_root` (the directory NACC manages).
    pub async fn reconcile(
        &self,
        repo: &nacc_git::GitRepository,
        worktrees_root: &Path,
        project_id: ProjectId,
    ) -> Result<ReconciliationReport> {
        let leases = self.leases_for_project(project_id).await?;
        let mut actions = Vec::new();
        // Every lease in the database, in any state and any project: the
        // "is this worktree managed?" question is global, even though the
        // "should I act on it?" question is per-project. Using only this
        // project's leases here would misreport another project's healthy
        // worktree as unmanaged -- a real bug this module's own scoping
        // test caught.
        let known_paths: Vec<PathBuf> = self
            .storage()
            .list_worktree_leases()
            .await?
            .into_iter()
            .map(|lease| PathBuf::from(lease.path))
            .collect();

        for lease in leases
            .into_iter()
            .filter(|lease| lease.state == nacc_domain::WorktreeState::Active)
        {
            actions.push(self.reconcile_one(repo, &lease).await?);
        }

        // Unmanaged worktrees: everything git knows about that lives under
        // the managed root but has no lease row. Reported, never removed --
        // deleting work NACC cannot account for is precisely the mistake
        // the quarantine rule exists to prevent.
        //
        // The comparison is done on canonicalized paths on purpose: git can
        // report a path through a different spelling than the caller used
        // (a symlinked temp directory, an 8.3 short name, `/` vs `\`), and
        // a lexicographic-`starts_with` check that misses the managed root
        // would silently skip every real worktree -- caught by this
        // module's own "unmanaged worktree" test failing against a real
        // git, not by reasoning about it.
        let managed_root =
            std::fs::canonicalize(worktrees_root).unwrap_or_else(|_| worktrees_root.to_path_buf());
        for info in repo.list_worktrees().await? {
            let candidate = std::fs::canonicalize(&info.path).unwrap_or_else(|_| info.path.clone());
            if !candidate.starts_with(&managed_root) {
                continue;
            }
            if known_paths
                .iter()
                .any(|known| paths_equal(known, &info.path))
            {
                continue;
            }
            tracing::warn!(
                path = %info.path.display(),
                "found a worktree under NACC's managed root with no lease record"
            );
            actions.push(ReconciliationAction::UnmanagedWorktree { path: info.path });
        }

        if !actions.is_empty() {
            tracing::info!(
                action_count = actions.len(),
                "worktree reconciliation complete"
            );
        }
        Ok(ReconciliationReport { actions })
    }

    async fn reconcile_one(
        &self,
        repo: &nacc_git::GitRepository,
        lease: &WorktreeLease,
    ) -> Result<ReconciliationAction> {
        if let Some(owner) = lease.owner_process_id {
            if nacc_process::process_alive(owner) {
                return Ok(ReconciliationAction::OwnerStillRunning {
                    lease_id: lease.id,
                    owner_process_id: owner,
                });
            }
        }

        let inspection = self.inspect(repo, lease).await?;
        if !inspection.path_exists {
            self.mark_released(lease, "reconciliation: worktree directory is absent")
                .await?;
            return Ok(ReconciliationAction::AlreadyAbsent {
                lease_id: lease.id,
                path: PathBuf::from(&lease.path),
            });
        }

        if inspection.must_preserve() {
            let reason = format!(
                "reconciliation after an unclean shutdown: owner process {:?} is no longer running",
                lease.owner_process_id
            );
            let (_updated, destination) = self.quarantine_lease(repo, lease, &reason).await?;
            return Ok(ReconciliationAction::Quarantined {
                lease_id: lease.id,
                destination,
                reason,
            });
        }

        let path = PathBuf::from(&lease.path);
        repo.remove_worktree(&path, false).await?;
        self.mark_released(lease, "reconciliation: clean worktree removed")
            .await?;
        Ok(ReconciliationAction::Released {
            lease_id: lease.id,
            path,
        })
    }
}

/// Convenience for callers that only care whether a release outcome
/// preserved work.
pub fn outcome_preserved(outcome: &ReleaseOutcome) -> bool {
    matches!(outcome, ReleaseOutcome::Quarantined { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::{WorktreeManager, WorktreeState};
    use nacc_domain::WorkflowRunId;
    use nacc_storage::Database;

    /// A pid that is definitely not alive: a real process that has already
    /// exited. Using a made-up number like 999_999 would be a guess about
    /// the OS's pid space; this is an observation.
    fn exited_process_id() -> u32 {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/C", "exit", "0"])
            .spawn()
            .expect("spawn a short-lived process");
        let pid = child.id();
        child.wait().expect("wait for it to exit");
        pid
    }

    #[tokio::test]
    async fn a_lease_owned_by_a_live_process_is_left_alone() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let project = ProjectId::new();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db);
        let lease = manager
            .allocate(&repo, request(project, &root, "Live Owner"))
            .await
            .unwrap();

        let report = manager
            .reconcile(&repo, &root.join(".nacc-worktrees"), project)
            .await
            .unwrap();

        assert_eq!(
            report.actions,
            vec![ReconciliationAction::OwnerStillRunning {
                lease_id: lease.id,
                owner_process_id: std::process::id(),
            }]
        );
        assert!(
            std::path::Path::new(&lease.path).is_dir(),
            "nothing may be moved while the owner lives"
        );
    }

    #[tokio::test]
    async fn a_dead_owners_dirty_worktree_is_quarantined_not_destroyed() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let project = ProjectId::new();
        let db = Database::open_in_memory().unwrap();
        let dead = exited_process_id();
        let manager = WorktreeManager::new(db).with_owner_process_id(dead);
        let lease = manager
            .allocate(&repo, request(project, &root, "Crashed Run"))
            .await
            .unwrap();
        std::fs::write(
            std::path::Path::new(&lease.path).join("half-done.rs"),
            "work\n",
        )
        .unwrap();

        let report = manager
            .reconcile(&repo, &root.join(".nacc-worktrees"), project)
            .await
            .unwrap();

        assert_eq!(report.quarantined(), 1);
        let destination = match &report.actions[0] {
            ReconciliationAction::Quarantined { destination, .. } => destination.clone(),
            other => panic!("expected a quarantine, got {other:?}"),
        };
        assert!(
            destination.join("half-done.rs").is_file(),
            "the crashed run's uncommitted file must still exist after quarantine"
        );
        assert!(destination.join("NACC-QUARANTINE.json").is_file());
    }

    #[tokio::test]
    async fn a_dead_owners_clean_worktree_is_released() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let project = ProjectId::new();
        let db = Database::open_in_memory().unwrap();
        let dead = exited_process_id();
        let manager = WorktreeManager::new(db.clone()).with_owner_process_id(dead);
        let lease = manager
            .allocate(&repo, request(project, &root, "Clean Run"))
            .await
            .unwrap();

        let report = manager
            .reconcile(&repo, &root.join(".nacc-worktrees"), project)
            .await
            .unwrap();

        assert_eq!(report.released(), 1);
        assert!(!std::path::Path::new(&lease.path).exists());
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
    async fn an_unmanaged_worktree_is_reported_and_never_touched() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let managed = root.join(".nacc-worktrees");
        let project = ProjectId::new();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db);

        // A worktree that exists in git under the managed root, with no
        // lease row at all -- e.g. a database was deleted, or a human made
        // it by hand.
        std::fs::create_dir_all(&managed).unwrap();
        let orphan = managed.join("by-hand");
        repo.add_worktree(&orphan, "nacc-by-hand", "HEAD")
            .await
            .unwrap();

        let report = manager.reconcile(&repo, &managed, project).await.unwrap();

        assert!(report.needs_attention());
        assert_eq!(report.unmanaged().len(), 1);
        assert!(
            orphan.is_dir(),
            "an unmanaged worktree must be reported, never removed"
        );
    }

    #[tokio::test]
    async fn started_runs_are_scoped_by_project() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let dead = exited_process_id();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db).with_owner_process_id(dead);

        let mine = ProjectId::new();
        let other = ProjectId::new();
        let mut lease = manager
            .allocate(&repo, request(mine, &root, "Mine"))
            .await
            .unwrap();
        lease.workflow_run_id = Some(WorkflowRunId::new());
        let theirs = manager
            .allocate(&repo, request(other, &root, "Theirs"))
            .await
            .unwrap();

        let report = manager
            .reconcile(&repo, &root.join(".nacc-worktrees"), mine)
            .await
            .unwrap();
        assert_eq!(
            report.actions.len(),
            1,
            "only this project's lease is reconciled"
        );
        assert!(
            std::path::Path::new(&theirs.path).is_dir(),
            "another project's worktree must be untouched"
        );
    }

    #[test]
    fn outcome_preserved_matches_only_quarantine() {
        assert!(outcome_preserved(&ReleaseOutcome::Quarantined {
            destination: PathBuf::from("x"),
            reason: "dirty".into(),
        }));
        assert!(!outcome_preserved(&ReleaseOutcome::Kept {
            reason: "policy".into()
        }));
    }
}
