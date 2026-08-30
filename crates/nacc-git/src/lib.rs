//! Typed Git operations invoked via the installed git CLI with argument
//! arrays, never interpolated shell strings (master plan S4.5, S13.3;
//! Phase 3 scope).
//!
//! This crate is a thin, typed wrapper around the `git` executable --
//! master plan S4.5's own reasoning for using the installed CLI rather
//! than a Rust Git library as the mutation engine: "it respects the
//! user's credential helpers, hooks, signing setup, filters, LFS
//! configuration, and existing Git behavior." Every subprocess is spawned
//! with an explicit argument array (never a shell string built by
//! concatenation) and, on Windows, `CREATE_NO_WINDOW` so a background
//! Git call never flashes a console window -- the same two rules the
//! Phase 0 foundation audit found upstream AgentPanel's own `git.rs`
//! already followed, carried forward here as a fresh implementation
//! (S5.2: "prefer new Rust workspace crates... over invasive edits to
//! upstream").
//!
//! `nacc-worktree` (branch/worktree *lifecycle* -- leases, drift
//! detection, cleanup policy) is built on top of this crate's primitives,
//! not merged into it -- matching the crate boundary the master plan's
//! workspace layout (S6) draws between "typed Git operations" and
//! "branch/worktree lifecycle".

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("failed to spawn git: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("git {args:?} failed (exit {exit_code:?}): {stderr}")]
    CommandFailed {
        args: Vec<String>,
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("{path:?} does not look like a Git repository (or git is not on PATH): {detail}")]
    NotARepository { path: PathBuf, detail: String },
    #[error("could not parse `git {context}` output: {detail}")]
    ParseError {
        context: &'static str,
        detail: String,
    },
}

pub type Result<T> = std::result::Result<T, GitError>;

/// A validated handle to an existing Git working tree or worktree at
/// `root`. Constructing one (`GitRepository::open`) confirms `root` is
/// really inside a Git work tree -- every other method assumes that
/// already holds.
#[derive(Clone, Debug)]
pub struct GitRepository {
    root: PathBuf,
}

/// One entry from `git worktree list --porcelain`, master plan S16's
/// worktree model made concrete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub head_commit: String,
    /// `None` on a detached-HEAD worktree (`is_detached` is then `true`)
    /// or the rare edge case of a brand-new repository with no commits
    /// yet.
    pub branch: Option<String>,
    pub is_bare: bool,
    pub is_detached: bool,
    pub is_locked: bool,
}

fn new_git_command(cwd: Option<&Path>) -> tokio::process::Command {
    let mut cmd = std::process::Command::new("git");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    // Windows-only: avoids a console window flashing for every background
    // git call (master plan S13.3's Windows-specific edge case list).
    // No-op on other platforms rather than `#[cfg(windows)]`-gating every
    // call site -- the master plan's own "Windows-first... later
    // Linux/macOS portability" (S1) means this crate should not hard-fail
    // to compile off Windows even though CI only ever runs it there.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    tokio::process::Command::from(cmd)
}

async fn run_git(cwd: Option<&Path>, args: &[&str]) -> Result<String> {
    let output = new_git_command(cwd).args(args).output().await?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            args: args.iter().map(|s| s.to_string()).collect(),
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The installed git's own version string (e.g. `"git version 2.53.0"`),
/// for the Setup Wizard's prerequisite-detection step (master plan
/// S17.1: "Show exact executable paths and versions").
pub async fn git_version() -> Result<String> {
    Ok(run_git(None, &["--version"]).await?.trim().to_string())
}

impl GitRepository {
    /// Open an existing Git working tree at `root`, failing with
    /// [`GitError::NotARepository`] if it is not one (or `git` is not on
    /// PATH at all).
    pub async fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        match run_git(Some(&root), &["rev-parse", "--is-inside-work-tree"]).await {
            Ok(out) if out.trim() == "true" => Ok(Self { root }),
            Ok(out) => Err(GitError::NotARepository {
                path: root,
                detail: format!("unexpected `git rev-parse --is-inside-work-tree` output: {out:?}"),
            }),
            Err(e) => Err(GitError::NotARepository {
                path: root,
                detail: e.to_string(),
            }),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The current branch name, or the literal string `"HEAD"` on a
    /// detached HEAD (git's own convention for `--abbrev-ref HEAD`) --
    /// callers that need to distinguish a detached HEAD reliably should
    /// use [`Self::list_worktrees`]'s `is_detached` field instead.
    pub async fn current_branch(&self) -> Result<String> {
        Ok(
            run_git(Some(&self.root), &["rev-parse", "--abbrev-ref", "HEAD"])
                .await?
                .trim()
                .to_string(),
        )
    }

    /// The full SHA of `HEAD`.
    pub async fn head_commit(&self) -> Result<String> {
        Ok(run_git(Some(&self.root), &["rev-parse", "HEAD"])
            .await?
            .trim()
            .to_string())
    }

    /// Every worktree attached to this repository, main checkout
    /// included (master plan S16).
    pub async fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let output = run_git(Some(&self.root), &["worktree", "list", "--porcelain"]).await?;
        parse_worktree_list(&output)
    }

    /// Create a new worktree at `path` on a new branch `branch`, based on
    /// `base` (a commit-ish: a branch name, tag, or SHA). Master plan
    /// S16 rule 4 ("branch names are deterministic, sanitized, and
    /// collision-safe") is the caller's responsibility -- see
    /// [`sanitize_branch_segment`] -- this method does not sanitize
    /// `branch` itself, since a caller that already has a valid,
    /// intentional branch name (e.g. resuming a known lease) must not
    /// have it silently rewritten.
    pub async fn add_worktree(&self, path: &Path, branch: &str, base: &str) -> Result<()> {
        let path_str = path.to_string_lossy().into_owned();
        run_git(
            Some(&self.root),
            &["worktree", "add", "-b", branch, &path_str, base],
        )
        .await?;
        Ok(())
    }

    /// Remove a worktree at `path`. Retries a bounded number of times on
    /// failure: a Windows file handle held by an editor, an antivirus
    /// scan, or a just-exited child process is a real, encountered
    /// transient failure mode (documented in the Phase 0 foundation
    /// audit's read of upstream AgentPanel's `git.rs`, which carries the
    /// same kind of fallback), not a theoretical one -- most such locks
    /// clear within a second.
    pub async fn remove_worktree(&self, path: &Path, force: bool) -> Result<()> {
        let path_str = path.to_string_lossy().into_owned();
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&path_str);

        const MAX_ATTEMPTS: u32 = 5;
        let mut last_err = None;
        for attempt in 1..=MAX_ATTEMPTS {
            match run_git(Some(&self.root), &args).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    tracing::debug!(
                        attempt,
                        max_attempts = MAX_ATTEMPTS,
                        path = %path.display(),
                        error = %e,
                        "worktree remove attempt failed, will retry if attempts remain"
                    );
                    last_err = Some(e);
                    if attempt < MAX_ATTEMPTS {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            200 * u64::from(attempt),
                        ))
                        .await;
                    }
                }
            }
        }
        Err(last_err.expect("loop runs at least once, so this is always Some after it exits"))
    }

    /// Whether `path` (normally a worktree of this repository) has any
    /// uncommitted changes -- master plan S16 rule 9's cleanup-safety
    /// check.
    pub async fn has_uncommitted_changes(&self, path: &Path) -> Result<bool> {
        let output = run_git(Some(path), &["status", "--porcelain"]).await?;
        Ok(!output.trim().is_empty())
    }

    /// Whether the branch checked out at `path` has commits not present
    /// on any remote-tracking branch -- master plan S16 rule 9's other
    /// cleanup-safety check ("unpushed commits").
    pub async fn has_unpushed_commits(&self, path: &Path) -> Result<bool> {
        // `@{push}` resolves to the branch's configured push target; if
        // none is configured (a fresh local-only branch), git errors --
        // which means by definition everything on it is "unpushed."
        match run_git(Some(path), &["rev-list", "@{push}..HEAD", "--count"]).await {
            Ok(out) => {
                let count: u64 = out.trim().parse().map_err(|e| GitError::ParseError {
                    context: "rev-list @{push}..HEAD --count",
                    detail: format!("{e}: {out:?}"),
                })?;
                Ok(count > 0)
            }
            Err(GitError::CommandFailed { .. }) => Ok(true),
            Err(e) => Err(e),
        }
    }
}

fn parse_worktree_list(porcelain: &str) -> Result<Vec<WorktreeInfo>> {
    #[derive(Default)]
    struct Building {
        path: Option<PathBuf>,
        head_commit: Option<String>,
        branch: Option<String>,
        is_bare: bool,
        is_detached: bool,
        is_locked: bool,
    }

    fn finish(b: Building) -> Result<Option<WorktreeInfo>> {
        let Some(path) = b.path else {
            return Ok(None);
        };
        let head_commit = b.head_commit.ok_or_else(|| GitError::ParseError {
            context: "worktree list --porcelain",
            detail: format!("worktree {} has no HEAD line", path.display()),
        })?;
        Ok(Some(WorktreeInfo {
            path,
            head_commit,
            branch: b.branch,
            is_bare: b.is_bare,
            is_detached: b.is_detached,
            is_locked: b.is_locked,
        }))
    }

    let mut out = Vec::new();
    let mut current = Building::default();

    for line in porcelain.lines() {
        if line.is_empty() {
            if let Some(info) = finish(std::mem::take(&mut current))? {
                out.push(info);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            current.path = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            current.head_commit = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("branch ") {
            current.branch = Some(rest.to_string());
        } else if line == "bare" {
            current.is_bare = true;
        } else if line == "detached" {
            current.is_detached = true;
        } else if line.starts_with("locked") {
            current.is_locked = true;
        }
        // Any other line (e.g. "prunable ...") is a real but
        // not-yet-modeled porcelain field -- ignored rather than treated
        // as a parse error, so a future git version adding new fields
        // does not break this parser.
    }
    if let Some(info) = finish(current)? {
        out.push(info);
    }
    Ok(out)
}

/// Sanitize a user-supplied or role-derived label into a valid, safe Git
/// branch-name *segment*: lowercase ASCII alphanumerics and single
/// hyphens, no leading/trailing hyphen, never empty. Deterministic --
/// the same input always sanitizes to the same output (master plan S16
/// rule 4) -- but not collision-safe by itself: combining this with a
/// unique lease identifier so two roles named the same thing don't
/// collide is `nacc-worktree`'s job (the lifecycle/policy layer), not
/// this crate's.
pub fn sanitize_branch_segment(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_sep = true; // suppress a leading '-'
    for ch in input.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('-');
            last_was_sep = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "task".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique, self-cleaning temp directory -- deliberately not the
    /// `tempfile`/`tempdir` crates, matching this workspace's existing
    /// convention (`nacc-observability`, `nacc-storage`) of a plain
    /// `std::env::temp_dir()` subdirectory rather than adding a new
    /// dependency for something this small.
    struct TempRepoDir(PathBuf);

    impl TempRepoDir {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("nacc-git-test-{label}-{nanos}-{n}"));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRepoDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn init_test_repo() -> (TempRepoDir, GitRepository) {
        let dir = TempRepoDir::new("repo");
        run_git(Some(dir.path()), &["init", "--initial-branch=main"])
            .await
            .expect("git init");
        run_git(
            Some(dir.path()),
            &["config", "user.email", "nacc-test@example.invalid"],
        )
        .await
        .expect("git config user.email");
        run_git(Some(dir.path()), &["config", "user.name", "NACC Test"])
            .await
            .expect("git config user.name");
        std::fs::write(dir.path().join("README.md"), "nacc-git test fixture\n")
            .expect("write README");
        run_git(Some(dir.path()), &["add", "."])
            .await
            .expect("git add");
        run_git(Some(dir.path()), &["commit", "-m", "initial commit"])
            .await
            .expect("git commit");

        let repo = GitRepository::open(dir.path())
            .await
            .expect("just-initialized directory must open as a repository");
        (dir, repo)
    }

    #[tokio::test]
    async fn opening_a_non_repository_directory_is_a_real_error() {
        let dir = TempRepoDir::new("not-a-repo");
        let result = GitRepository::open(dir.path()).await;
        assert!(matches!(result, Err(GitError::NotARepository { .. })));
    }

    #[tokio::test]
    async fn opening_a_freshly_initialized_repository_succeeds() {
        let (_dir, repo) = init_test_repo().await;
        let head = repo.head_commit().await.unwrap();
        assert_eq!(
            head.len(),
            40,
            "a full SHA-1 commit hash is 40 hex characters"
        );
    }

    #[tokio::test]
    async fn current_branch_reports_the_initial_branch_name() {
        let (_dir, repo) = init_test_repo().await;
        assert_eq!(repo.current_branch().await.unwrap(), "main");
    }

    #[tokio::test]
    async fn list_worktrees_reports_the_main_checkout() {
        let (dir, repo) = init_test_repo().await;
        let worktrees = repo.list_worktrees().await.unwrap();
        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].branch.as_deref(), Some("refs/heads/main"));
        assert!(!worktrees[0].is_detached);
        assert_eq!(
            std::fs::canonicalize(&worktrees[0].path).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[tokio::test]
    async fn add_list_and_remove_worktree_round_trips() {
        let (dir, repo) = init_test_repo().await;
        let worktree_path = dir.path().join("nacc-test-worktree");

        repo.add_worktree(&worktree_path, "nacc-test-branch", "main")
            .await
            .expect("add_worktree should succeed against a real repository");

        let worktrees = repo.list_worktrees().await.unwrap();
        assert_eq!(worktrees.len(), 2, "main checkout plus the new worktree");
        let added = worktrees
            .iter()
            .find(|w| w.branch.as_deref() == Some("refs/heads/nacc-test-branch"))
            .expect("the newly added worktree must be listed");
        assert!(worktree_path.ends_with(added.path.file_name().unwrap()));

        assert!(
            !repo.has_uncommitted_changes(&worktree_path).await.unwrap(),
            "a freshly added worktree has no uncommitted changes"
        );

        repo.remove_worktree(&worktree_path, true)
            .await
            .expect("remove_worktree should succeed");

        let worktrees_after = repo.list_worktrees().await.unwrap();
        assert_eq!(worktrees_after.len(), 1, "back to just the main checkout");
    }

    #[tokio::test]
    async fn has_uncommitted_changes_detects_a_new_file() {
        let (dir, repo) = init_test_repo().await;
        std::fs::write(dir.path().join("untracked.txt"), "hello").unwrap();
        assert!(repo.has_uncommitted_changes(dir.path()).await.unwrap());
    }

    #[tokio::test]
    async fn has_unpushed_commits_is_true_for_a_branch_with_no_push_target() {
        // No remote is configured in this fixture at all, so `@{push}`
        // cannot resolve -- by definition, everything on the branch is
        // unpushed. Real behavior against a real git, not a stub.
        let (_dir, repo) = init_test_repo().await;
        assert!(repo.has_unpushed_commits(repo.root()).await.unwrap());
    }

    #[test]
    fn sanitize_branch_segment_lowercases_and_hyphenates() {
        assert_eq!(sanitize_branch_segment("Fix Login Bug!!"), "fix-login-bug");
    }

    #[test]
    fn sanitize_branch_segment_is_deterministic() {
        let a = sanitize_branch_segment("Refactor / Migration Specialist");
        let b = sanitize_branch_segment("Refactor / Migration Specialist");
        assert_eq!(a, b);
    }

    #[test]
    fn sanitize_branch_segment_never_produces_leading_or_trailing_hyphens() {
        let sanitized = sanitize_branch_segment("--- weird ___ input ---");
        assert!(!sanitized.starts_with('-'));
        assert!(!sanitized.ends_with('-'));
    }

    #[test]
    fn sanitize_branch_segment_of_an_all_symbol_input_is_never_empty() {
        assert_eq!(sanitize_branch_segment("!!!"), "task");
    }

    #[tokio::test]
    async fn git_version_reports_a_real_version_string() {
        let version = git_version().await.expect("git must be installed in CI");
        assert!(version.to_lowercase().contains("git version"));
    }

    #[test]
    fn worktree_list_porcelain_parses_a_realistic_multi_worktree_sample() {
        let sample = "worktree /repo\nHEAD aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nbranch refs/heads/main\n\nworktree /repo/.nacc-worktrees/reviewer-1\nHEAD bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\ndetached\n\nworktree /repo/.nacc-worktrees/locked-1\nHEAD cccccccccccccccccccccccccccccccccccccccc\nbranch refs/heads/locked-branch\nlocked reason: in use\n\n";
        let parsed = parse_worktree_list(sample).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].branch.as_deref(), Some("refs/heads/main"));
        assert!(parsed[1].is_detached);
        assert!(parsed[1].branch.is_none());
        assert!(parsed[2].is_locked);
    }
}
