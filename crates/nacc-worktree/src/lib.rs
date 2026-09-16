//! Git worktree lease lifecycle: allocation, drift detection,
//! quarantine-not-destroy cleanup, and crash reconciliation (master plan
//! S11, S16, S17.9 -- Phase 3 scope).
//!
//! # Why a lease record at all
//!
//! `git worktree list` says what exists. It does not say which workflow run
//! created a worktree, what commit it was branched from, whether the
//! process that owned it is still alive, or whether its work was already
//! integrated. Master plan S16 requires all of that, and S17.15 requires a
//! startup reconciliation pass over it -- so the durable half
//! ([`nacc_domain::WorktreeLease`], `nacc-storage`'s
//! `worktree_leases` table) carries the facts Git does not.
//!
//! # The two rules this crate exists to enforce
//!
//! 1. **A dirty or unpushed worktree is never destroyed.** If a worktree
//!    holds uncommitted or unpushed work when its run ends, it is
//!    *quarantined*: moved aside into a `.quarantine` directory with a
//!    JSON manifest recording what it was and why, and the lease is marked
//!    `Quarantined`. A human (or a future restore action) can still get the
//!    work back.
//! 2. **Nothing outside the managed root is touched.** Allocation creates
//!    paths under the caller-supplied worktrees root only; the repository's
//!    own checkout is never a cleanup target.
//!
//! Allocation is branch-deterministic and collision-safe (S16 rule 4): the
//! branch is `nacc/<sanitized-label>-<8 hex of the lease id>`, so two roles
//! with the same label never collide, and the same lease is always
//! reconstructible from its own id.

use std::path::{Path, PathBuf};

use nacc_domain::{
    NodeRunId, ProjectId, WorkflowRunId, WorktreeLease, WorktreeLeaseId, WorktreeState,
};
use nacc_git::{sanitize_branch_segment, GitRepository};
use nacc_storage::Database;

mod reconcile;
mod release;

pub use reconcile::{outcome_preserved, ReconciliationAction, ReconciliationReport};
pub use release::{ReleaseOutcome, ReleasePolicy, ReleaseReport};

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("git error: {0}")]
    Git(#[from] nacc_git::GitError),
    #[error("storage error: {0}")]
    Storage(#[from] nacc_storage::StorageError),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to serialize the quarantine manifest: {0}")]
    Manifest(#[from] serde_json::Error),
    #[error(
        "worktrees root {root:?} is not an absolute path; NACC only manages worktrees it can \
         identify unambiguously"
    )]
    RootNotAbsolute { root: PathBuf },
    #[error("worktree {path:?} did not appear in `git worktree list` after allocation")]
    WorktreeNotRegistered { path: PathBuf },
    #[error("refusing to quarantine {path:?}: the destination {destination:?} already exists")]
    QuarantineTargetExists { path: PathBuf, destination: PathBuf },
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, WorktreeError>;

/// What the caller wants allocated.
#[derive(Clone, Debug)]
pub struct AllocateRequest {
    pub project_id: ProjectId,
    pub workflow_run_id: Option<WorkflowRunId>,
    pub node_run_id: Option<NodeRunId>,
    /// Human-readable label (normally the Role Matrix row's role name).
    /// Sanitized into the branch and directory names.
    pub label: String,
    /// Commit-ish to branch from -- `"HEAD"`, a branch name, or a SHA. It
    /// is *resolved* to a full SHA before the lease is written, so drift
    /// detection compares against a fixed point rather than a moving
    /// branch name.
    pub base: String,
    /// Directory NACC manages worktrees under (typically
    /// `<project>/.nacc-worktrees`).
    pub worktrees_root: PathBuf,
}

/// One drift fact discovered by [`WorktreeManager::inspect`]. A closed
/// vocabulary, like every other cross-layer signal in this workspace: an
/// adapter or UI that sees a variant it does not know is a code-change
/// signal, not something to pass through as a string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorktreeDrift {
    /// The lease says a worktree exists here; the filesystem disagrees.
    MissingPath,
    /// The path exists but Git no longer has it registered as a worktree.
    NotRegistered,
    /// A different branch is checked out than the lease recorded.
    BranchChanged { expected: String, actual: String },
    /// HEAD moved since the lease last observed it (an agent committed, or
    /// something reset the tree).
    HeadMoved { recorded: String, current: String },
    /// Uncommitted changes are present.
    DirtyWorkingTree,
    /// The worktree has `count` commits not reachable from its recorded
    /// base -- i.e. real work that exists nowhere else yet, whether or not
    /// a remote happens to be configured. Deliberately *not* phrased as
    /// "unpushed": a fresh worktree branch with no remote configured is
    /// trivially "unpushed" while holding no unintegrated work at all,
    /// which would make every cleanup decision needlessly preserve
    /// everything.
    UnintegratedCommits { count: u32 },
    /// The worktree is on a detached HEAD.
    DetachedHead,
}

impl WorktreeDrift {
    /// Whether this drift means the work must not be discarded.
    pub fn requires_preservation(&self) -> bool {
        matches!(
            self,
            WorktreeDrift::DirtyWorkingTree | WorktreeDrift::UnintegratedCommits { .. }
        )
    }
}

/// The result of inspecting one lease against the real repository.
#[derive(Clone, Debug)]
pub struct LeaseInspection {
    pub lease_id: WorktreeLeaseId,
    pub path_exists: bool,
    pub registered: bool,
    pub current_branch: Option<String>,
    pub current_head: Option<String>,
    pub drift: Vec<WorktreeDrift>,
}

impl LeaseInspection {
    pub fn is_healthy(&self) -> bool {
        self.drift.is_empty()
    }

    /// Whether releasing this lease safely means quarantining it rather
    /// than removing it (master plan S16 rule 9).
    pub fn must_preserve(&self) -> bool {
        !self.path_exists || self.drift.iter().any(WorktreeDrift::requires_preservation)
    }
}

/// Owns NACC's worktree lifecycle for one database. Cheap to clone-free:
/// constructed once in the composition root (like `Database` itself).
pub struct WorktreeManager {
    storage: Database,
    /// Recorded on every lease this manager allocates, so startup
    /// reconciliation can tell "somebody is still using this worktree"
    /// from "NACC crashed and left it behind".
    owner_process_id: u32,
    /// Exact 8-hex-char id fragment used in branch/directory names. Short
    /// enough to stay readable, long enough that two live leases colliding
    /// requires an actual UUID collision.
    id_fragment_len: usize,
}

impl WorktreeManager {
    pub fn new(storage: Database) -> Self {
        Self {
            storage,
            owner_process_id: std::process::id(),
            id_fragment_len: 8,
        }
    }

    /// Override the recorded owner process id (tests need to simulate a
    /// lease whose owner is long gone).
    pub fn with_owner_process_id(mut self, pid: u32) -> Self {
        self.owner_process_id = pid;
        self
    }

    pub fn storage(&self) -> &Database {
        &self.storage
    }

    /// The deterministic branch name for a lease: `nacc/<label>-<fragment>`.
    /// Deterministic *and* collision-safe, which is the pair of properties
    /// master plan S16 rule 4 asks for.
    pub fn branch_name(&self, label: &str, lease_id: WorktreeLeaseId) -> String {
        format!(
            "nacc/{}-{}",
            sanitize_branch_segment(label),
            self.id_fragment(lease_id)
        )
    }

    /// The deterministic directory name for a lease.
    pub fn worktree_dir_name(&self, label: &str, lease_id: WorktreeLeaseId) -> String {
        format!(
            "{}-{}",
            sanitize_branch_segment(label),
            self.id_fragment(lease_id)
        )
    }

    fn id_fragment(&self, lease_id: WorktreeLeaseId) -> String {
        let hex = lease_id.as_uuid().simple().to_string();
        hex[..self.id_fragment_len].to_string()
    }

    /// Create a worktree, write its lease, and return it. The lease is
    /// persisted *before* it is returned, so a crash immediately after
    /// allocation still leaves a record on disk for reconciliation.
    pub async fn allocate(
        &self,
        repo: &GitRepository,
        request: AllocateRequest,
    ) -> Result<WorktreeLease> {
        if !request.worktrees_root.is_absolute() {
            return Err(WorktreeError::RootNotAbsolute {
                root: request.worktrees_root.clone(),
            });
        }

        let lease_id = WorktreeLeaseId::new();
        let branch = self.branch_name(&request.label, lease_id);
        let dir_name = self.worktree_dir_name(&request.label, lease_id);
        let path = request.worktrees_root.join(&dir_name);

        // Resolve the base to a fixed SHA *before* creating the worktree:
        // recording "main" would leave drift detection comparing against a
        // moving target.
        let base = if request.base.trim().is_empty() {
            "HEAD".to_string()
        } else {
            request.base.clone()
        };
        let base_commit = repo.resolve_commit(&base).await?;

        std::fs::create_dir_all(&request.worktrees_root)?;
        repo.add_worktree(&path, &branch, &base_commit).await?;

        let now = now_millis();
        let lease = WorktreeLease {
            id: lease_id,
            project_id: request.project_id,
            workflow_run_id: request.workflow_run_id,
            node_run_id: request.node_run_id,
            path: path.to_string_lossy().into_owned(),
            branch: branch.clone(),
            base_commit: base_commit.clone(),
            head_commit: Some(base_commit),
            state: WorktreeState::Active,
            owner_process_id: Some(self.owner_process_id),
            quarantine_reason: None,
            created_at_millis: now,
            updated_at_millis: now,
        };
        self.storage.insert_worktree_lease(&lease).await?;

        tracing::info!(
            lease_id = %lease.id,
            branch = %branch,
            path = %path.display(),
            base_commit = %lease.base_commit,
            "allocated worktree lease"
        );
        Ok(lease)
    }

    /// Compare a lease against the real repository and working tree.
    /// Read-only: callers that want the observation persisted
    /// (`head_commit` refreshed) call [`Self::record_inspection`].
    pub async fn inspect(
        &self,
        repo: &GitRepository,
        lease: &WorktreeLease,
    ) -> Result<LeaseInspection> {
        let path = PathBuf::from(&lease.path);
        let mut drift = Vec::new();

        let path_exists = path.exists();
        if !path_exists {
            drift.push(WorktreeDrift::MissingPath);
            return Ok(LeaseInspection {
                lease_id: lease.id,
                path_exists,
                registered: false,
                current_branch: None,
                current_head: None,
                drift,
            });
        }

        let worktrees = repo.list_worktrees().await?;
        let registered = worktrees.iter().find(|w| paths_equal(&w.path, &path));
        let (current_branch, current_head, registered) = match registered {
            Some(info) => (
                info.branch
                    .as_deref()
                    .map(|b| b.strip_prefix("refs/heads/").unwrap_or(b).to_string()),
                Some(info.head_commit.clone()),
                true,
            ),
            None => (None, None, false),
        };
        if !registered {
            drift.push(WorktreeDrift::NotRegistered);
        }
        if let Some(branch) = &current_branch {
            if branch != &lease.branch {
                drift.push(WorktreeDrift::BranchChanged {
                    expected: lease.branch.clone(),
                    actual: branch.clone(),
                });
            }
        } else if registered {
            drift.push(WorktreeDrift::DetachedHead);
        }

        let recorded_head = lease
            .head_commit
            .clone()
            .unwrap_or_else(|| lease.base_commit.clone());
        if let Some(current) = &current_head {
            if current != &recorded_head {
                drift.push(WorktreeDrift::HeadMoved {
                    recorded: recorded_head,
                    current: current.clone(),
                });
            }
        }

        if repo.has_uncommitted_changes(&path).await? {
            drift.push(WorktreeDrift::DirtyWorkingTree);
        }
        match repo
            .count_commits_ahead_of(&path, &lease.base_commit)
            .await?
        {
            0 => {}
            count => drift.push(WorktreeDrift::UnintegratedCommits { count }),
        }

        Ok(LeaseInspection {
            lease_id: lease.id,
            path_exists,
            registered,
            current_branch,
            current_head,
            drift,
        })
    }

    /// Persist what an inspection observed (`head_commit`), so a restart
    /// does not report the same `HeadMoved` drift forever.
    pub async fn record_inspection(
        &self,
        lease: &WorktreeLease,
        inspection: &LeaseInspection,
    ) -> Result<WorktreeLease> {
        let mut updated = lease.clone();
        if let Some(head) = &inspection.current_head {
            updated.head_commit = Some(head.clone());
        }
        if let Some(branch) = &inspection.current_branch {
            updated.branch = branch.clone();
        }
        updated.updated_at_millis = now_millis();
        self.storage.update_worktree_lease(&updated).await?;
        Ok(updated)
    }

    /// Every active lease, across projects -- what the app reconciles on
    /// startup (master plan S17.15).
    pub async fn active_leases(&self) -> Result<Vec<WorktreeLease>> {
        Ok(self.storage.list_active_worktree_leases().await?)
    }

    /// Leases for one project.
    pub async fn leases_for_project(&self, project_id: ProjectId) -> Result<Vec<WorktreeLease>> {
        Ok(self
            .storage
            .list_worktree_leases_for_project(project_id)
            .await?)
    }
}

pub(crate) fn paths_equal(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

pub(crate) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let nanos = now_millis();
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("nacc-worktree-test-{label}-{nanos}-{n}"));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A real repository with one real commit, via the same typed
    /// operations the app itself uses (`init_repository`,
    /// `configure_identity`, `stage_all`, `commit`) -- no test-only Git
    /// escape hatch.
    pub async fn init_repo() -> (TempDir, GitRepository) {
        let dir = TempDir::new("repo");
        let repo = nacc_git::init_repository(dir.path(), "main")
            .await
            .expect("git init");
        repo.configure_identity("nacc-test@example.invalid", "NACC Test")
            .await
            .expect("git config");
        std::fs::write(dir.path().join("README.md"), "nacc-worktree fixture\n").expect("write");
        repo.stage_all().await.expect("git add");
        repo.commit("initial commit").await.expect("git commit");
        (dir, repo)
    }

    pub fn request(project_id: ProjectId, root: &Path, label: &str) -> AllocateRequest {
        AllocateRequest {
            project_id,
            workflow_run_id: Some(WorkflowRunId::new()),
            node_run_id: None,
            label: label.to_string(),
            base: "HEAD".to_string(),
            worktrees_root: root.join(".nacc-worktrees"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[tokio::test]
    async fn allocate_creates_a_real_worktree_and_persists_its_lease() {
        let (_dir, repo) = init_repo().await;
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db.clone());
        let root = repo.root().to_path_buf();

        let lease = manager
            .allocate(&repo, request(ProjectId::new(), &root, "Backend Implementer"))
            .await
            .expect("allocation against a real repository must succeed");

        assert!(PathBuf::from(&lease.path).is_dir(), "worktree must exist");
        assert_eq!(lease.state, WorktreeState::Active);
        assert!(
            lease.branch.starts_with("nacc/backend-implementer-"),
            "branch must be sanitized and id-suffixed: {}",
            lease.branch
        );
        assert_eq!(lease.base_commit.len(), 40);

        let stored = db
            .get_worktree_lease(lease.id)
            .await
            .unwrap()
            .expect("the lease must be durable, not just returned");
        assert_eq!(stored.path, lease.path);
        assert_eq!(stored.owner_process_id, Some(std::process::id()));
    }

    #[tokio::test]
    async fn the_same_label_twice_never_collides() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let project = ProjectId::new();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db);

        let a = manager
            .allocate(&repo, request(project, &root, "Reviewer"))
            .await
            .unwrap();
        let b = manager
            .allocate(&repo, request(project, &root, "Reviewer"))
            .await
            .unwrap();

        assert_ne!(a.branch, b.branch, "branch names must be collision-safe");
        assert_ne!(a.path, b.path, "worktree paths must be collision-safe");
        assert!(PathBuf::from(&a.path).is_dir() && PathBuf::from(&b.path).is_dir());
    }

    #[tokio::test]
    async fn allocation_rejects_a_relative_worktrees_root() {
        let (_dir, repo) = init_repo().await;
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db);
        let mut req = request(ProjectId::new(), Path::new("."), "relative");
        req.worktrees_root = PathBuf::from("relative/.nacc-worktrees");

        let err = manager.allocate(&repo, req).await.unwrap_err();
        assert!(matches!(err, WorktreeError::RootNotAbsolute { .. }));
    }

    #[tokio::test]
    async fn a_freshly_allocated_lease_inspects_as_healthy() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db);
        let lease = manager
            .allocate(&repo, request(ProjectId::new(), &root, "Explorer"))
            .await
            .unwrap();

        let inspection = manager.inspect(&repo, &lease).await.unwrap();
        assert!(inspection.path_exists);
        assert!(inspection.registered);
        assert_eq!(inspection.current_branch.as_deref(), Some(lease.branch.as_str()));
        assert!(
            inspection.drift.is_empty(),
            "unexpected drift: {:?}",
            inspection.drift
        );
        assert!(!inspection.must_preserve());
    }

    #[tokio::test]
    async fn inspect_reports_head_movement_and_a_dirty_tree() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db);
        let lease = manager
            .allocate(&repo, request(ProjectId::new(), &root, "Implementer"))
            .await
            .unwrap();

        let worktree = PathBuf::from(&lease.path);
        std::fs::write(worktree.join("agent-output.txt"), "work in progress\n").unwrap();
        let dirty = manager.inspect(&repo, &lease).await.unwrap();
        assert!(dirty.drift.contains(&WorktreeDrift::DirtyWorkingTree));
        assert!(dirty.must_preserve());
        assert!(
            !dirty
                .drift
                .iter()
                .any(|d| matches!(d, WorktreeDrift::UnintegratedCommits { .. })),
            "an uncommitted file is not yet a commit, so nothing is unintegrated"
        );

        // Commit the change (through the real typed API), then re-inspect:
        // HEAD has moved and the tree is clean again.
        let worktree_repo = GitRepository::open(&worktree).await.unwrap();
        worktree_repo.configure_identity("a@b.invalid", "Agent").await.unwrap();
        worktree_repo.stage_all().await.unwrap();
        worktree_repo.commit("agent commit").await.unwrap();

        let moved = manager.inspect(&repo, &lease).await.unwrap();
        assert!(
            moved
                .drift
                .iter()
                .any(|d| matches!(d, WorktreeDrift::HeadMoved { .. })),
            "committing in the worktree must show up as HeadMoved: {:?}",
            moved.drift
        );
        assert!(!moved.drift.contains(&WorktreeDrift::DirtyWorkingTree));
        assert!(
            moved
                .drift
                .contains(&WorktreeDrift::UnintegratedCommits { count: 1 }),
            "the agent's commit is unintegrated work that must be preserved: {:?}",
            moved.drift
        );
        assert!(moved.must_preserve());
    }

    #[tokio::test]
    async fn inspect_reports_a_missing_path_without_touching_git() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db);
        let lease = manager
            .allocate(&repo, request(ProjectId::new(), &root, "Gone"))
            .await
            .unwrap();

        std::fs::remove_dir_all(&lease.path).unwrap();
        let inspection = manager.inspect(&repo, &lease).await.unwrap();
        assert!(!inspection.path_exists);
        assert_eq!(inspection.drift, vec![WorktreeDrift::MissingPath]);
        assert!(inspection.must_preserve(), "a missing worktree is never 'clean'");
    }

    #[tokio::test]
    async fn record_inspection_persists_the_observed_head() {
        let (_dir, repo) = init_repo().await;
        let root = repo.root().to_path_buf();
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db.clone());
        let lease = manager
            .allocate(&repo, request(ProjectId::new(), &root, "Recorder"))
            .await
            .unwrap();

        let worktree = PathBuf::from(&lease.path);
        std::fs::write(worktree.join("x.txt"), "x\n").unwrap();
        let worktree_repo = GitRepository::open(&worktree).await.unwrap();
        worktree_repo.configure_identity("a@b.invalid", "Agent").await.unwrap();
        worktree_repo.stage_all().await.unwrap();
        let new_head = worktree_repo.commit("change").await.unwrap();

        let inspection = manager.inspect(&repo, &lease).await.unwrap();
        let updated = manager.record_inspection(&lease, &inspection).await.unwrap();
        assert_eq!(updated.head_commit.as_deref(), Some(new_head.as_str()));

        let stored = db.get_worktree_lease(lease.id).await.unwrap().unwrap();
        assert_eq!(stored.head_commit.as_deref(), Some(new_head.as_str()));

        // And now the drift is gone, because the recorded baseline moved.
        let re_inspected = manager.inspect(&repo, &stored).await.unwrap();
        assert!(!re_inspected
            .drift
            .iter()
            .any(|d| matches!(d, WorktreeDrift::HeadMoved { .. })));
    }

    #[test]
    fn branch_names_are_deterministic_for_the_same_lease_id() {
        let db = Database::open_in_memory().unwrap();
        let manager = WorktreeManager::new(db);
        let id = WorktreeLeaseId::new();
        assert_eq!(
            manager.branch_name("Fix Login Bug!!", id),
            manager.branch_name("Fix Login Bug!!", id)
        );
        assert!(manager.branch_name("Fix Login Bug!!", id).starts_with("nacc/fix-login-bug-"));
    }

    #[test]
    fn drift_preservation_classification_is_explicit() {
        assert!(WorktreeDrift::DirtyWorkingTree.requires_preservation());
        assert!(WorktreeDrift::UnintegratedCommits { count: 2 }.requires_preservation());
        assert!(!WorktreeDrift::HeadMoved {
            recorded: "a".into(),
            current: "b".into()
        }
        .requires_preservation());
        assert!(!WorktreeDrift::MissingPath.requires_preservation());
    }
}
