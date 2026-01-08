//! Shared test utilities for VCS tests.
//!
//! Provides RepoGuard and JjRepoGuard for creating temporary test repositories.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Run a git command in a directory.
pub fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("failed to spawn git");
    assert!(status.success(), "git command failed: {:?}", args);
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
    pub fn new() -> Self {
        // Handle poisoned mutex (from previous panics in tests)
        let lock = match cwd_lock().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let original = env::current_dir().expect("failed to get cwd");
        let dir = make_temp_dir("lumen-test");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);
        fs::write(dir.join("README.md"), "hello\n").expect("failed to write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);

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
