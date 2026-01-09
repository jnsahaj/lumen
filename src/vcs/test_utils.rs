//! Shared test utilities for VCS tests.
//!
//! Provides RepoGuard and JjRepoGuard for creating temporary test repositories.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Repository, Signature};

/// Global lock for tests that change the current working directory.
/// Prevents concurrent tests from interfering with each other.
pub fn cwd_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Create a unique temporary directory for tests.
pub fn make_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH - clock misconfigured")
        .as_nanos();
    let dir = env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), nanos));
    fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("failed to create temp dir at {:?}: {}", dir, e));
    dir
}

/// Run a git command in a directory using git2.
/// Supports common operations: init, config, add, commit, checkout, log.
/// For unsupported operations, falls back to CLI.
pub fn git(dir: &Path, args: &[&str]) {
    if args.is_empty() {
        panic!("git() called with empty args");
    }

    match args[0] {
        "init" => {
            Repository::init(dir).expect("failed to init repo");
        }
        "config" if args.len() >= 3 => {
            let repo = Repository::open(dir).expect("failed to open repo");
            let mut config = repo.config().expect("failed to get config");
            config
                .set_str(args[1], args[2])
                .expect("failed to set config");
        }
        "add" if args.len() >= 2 => {
            let repo = Repository::open(dir).expect("failed to open repo");
            let mut index = repo.index().expect("failed to get index");
            if args[1] == "." {
                index
                    .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
                    .expect("failed to add all files");
            } else {
                // Add specific file(s)
                for arg in &args[1..] {
                    index.add_path(Path::new(arg)).expect("failed to add file");
                }
            }
            index.write().expect("failed to write index");
        }
        "commit" if args.len() >= 3 && args[1] == "-m" => {
            let repo = Repository::open(dir).expect("failed to open repo");
            let sig = Signature::now("Test User", "test@example.com")
                .expect("failed to create signature");
            let mut index = repo.index().expect("failed to get index");
            let tree_oid = index.write_tree().expect("failed to write tree");
            let tree = repo.find_tree(tree_oid).expect("failed to find tree");

            let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            let parents: Vec<&git2::Commit> = parent.iter().collect();

            repo.commit(Some("HEAD"), &sig, &sig, args[2], &tree, &parents)
                .expect("failed to create commit");
        }
        "commit" if args.contains(&"--allow-empty") => {
            let repo = Repository::open(dir).expect("failed to open repo");
            let sig = Signature::now("Test User", "test@example.com")
                .expect("failed to create signature");
            let parent = repo
                .head()
                .ok()
                .and_then(|h| h.peel_to_commit().ok())
                .expect("no HEAD for empty commit");
            let tree = parent.tree().expect("failed to get tree");

            let msg = args
                .iter()
                .skip_while(|a| *a != &"-m")
                .nth(1)
                .unwrap_or(&"empty");

            repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[&parent])
                .expect("failed to create commit");
        }
        "checkout" if args.len() >= 3 && args[1] == "-b" => {
            let repo = Repository::open(dir).expect("failed to open repo");
            let head = repo.head().expect("failed to get HEAD");
            let commit = head.peel_to_commit().expect("failed to get commit");
            repo.branch(args[2], &commit, false)
                .expect("failed to create branch");
            let refname = format!("refs/heads/{}", args[2]);
            repo.set_head(&refname).expect("failed to set HEAD");
        }
        "checkout" if args.len() >= 2 => {
            let repo = Repository::open(dir).expect("failed to open repo");
            let refname = format!("refs/heads/{}", args[1]);
            repo.set_head(&refname).expect("failed to set HEAD");
            repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
                .expect("failed to checkout");
        }
        _ => {
            panic!(
                "Unsupported git command in test: {:?}. Add git2 implementation to test_utils::git()",
                args
            );
        }
    }
}

/// Run a jj command in a directory. Returns success status.
#[cfg(feature = "jj")]
pub fn jj(dir: &Path, args: &[&str]) -> bool {
    Command::new("jj")
        .current_dir(dir)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// RAII guard for a temporary git repository.
/// Creates a git repo, changes to it, and cleans up on drop.
pub struct RepoGuard {
    _lock: MutexGuard<'static, ()>,
    pub dir: PathBuf,
    original: PathBuf,
}

impl RepoGuard {
    /// Create a new temporary git repository with an initial commit.
    /// Uses git2 for repo initialization and commit creation.
    pub fn new() -> Self {
        // Handle poisoned mutex (from previous panics in tests)
        let lock = match cwd_lock().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let original = env::current_dir().expect("failed to get cwd");
        let dir = make_temp_dir("lumen-test");

        // Initialize repo with git2
        let repo = Repository::init(&dir).expect("failed to init repo");

        // Set config
        let mut config = repo.config().expect("failed to get config");
        config
            .set_str("user.email", "test@example.com")
            .expect("failed to set email");
        config
            .set_str("user.name", "Test User")
            .expect("failed to set name");

        // Create README.md file
        fs::write(dir.join("README.md"), "hello\n").expect("failed to write file");

        // Stage file
        let mut index = repo.index().expect("failed to get index");
        index
            .add_path(Path::new("README.md"))
            .expect("failed to add file");
        index.write().expect("failed to write index");
        let tree_oid = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_oid).expect("failed to find tree");

        // Create initial commit
        let sig =
            Signature::now("Test User", "test@example.com").expect("failed to create signature");
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "init",
            &tree,
            &[], // No parents for initial commit
        )
        .expect("failed to create commit");

        // Need to also set up main branch ref (git2 doesn't auto-create it on first commit like CLI)
        // The commit above should have already created HEAD pointing to the commit

        env::set_current_dir(&dir).expect("failed to set cwd");

        Self {
            _lock: lock,
            dir,
            original,
        }
    }
}

impl Drop for RepoGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// RAII guard for a temporary jj repository.
/// Returns None if jj is not available.
#[cfg(feature = "jj")]
pub struct JjRepoGuard {
    _lock: MutexGuard<'static, ()>,
    pub dir: PathBuf,
    original: PathBuf,
}

#[cfg(feature = "jj")]
impl JjRepoGuard {
    /// Create a new temporary jj repository.
    /// Returns None if jj CLI is not available.
    pub fn new() -> Option<Self> {
        // Handle poisoned mutex (from previous panics in tests)
        let lock = match cwd_lock().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let original = env::current_dir().expect("failed to get cwd");
        let dir = make_temp_dir("lumen-jj-test");

        // Initialize jj repo using CLI (for test setup only)
        if !jj(&dir, &["git", "init"]) {
            // jj not available, skip test
            return None;
        }

        jj(
            &dir,
            &["config", "set", "--repo", "user.email", "test@example.com"],
        );
        jj(&dir, &["config", "set", "--repo", "user.name", "Test User"]);

        fs::write(dir.join("README.md"), "hello\n").expect("failed to write file");

        // Force jj to snapshot the working copy so files are tracked
        jj(&dir, &["status"]);

        env::set_current_dir(&dir).expect("failed to set cwd");

        Some(Self {
            _lock: lock,
            dir,
            original,
        })
    }
}

#[cfg(feature = "jj")]
impl Drop for JjRepoGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original);
        let _ = fs::remove_dir_all(&self.dir);
    }
}
