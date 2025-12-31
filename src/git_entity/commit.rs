use crate::error::LumenError;
use std::process::Command;
use thiserror::Error;

use super::GIT_DIFF_EXCLUSIONS;

/// Errors that can occur when resolving commit metadata or diffs.
#[derive(Error, Debug, Clone)]
pub enum CommitError {
    #[error("Commit '{0}' not found")]
    InvalidCommit(String),

    #[error("Diff for commit '{0}' is empty")]
    EmptyDiff(String),
}

/// Parsed commit metadata and its diff content.
#[derive(Clone, Debug)]
pub struct Commit {
    pub full_hash: String,
    pub message: String,
    pub diff: String,
    pub author_name: String,
    pub author_email: String,
    pub date: String,
}

impl Commit {
    /// Build a commit object from a SHA or ref.
    pub fn new(sha: String) -> Result<Self, LumenError> {
        let sha = sha.trim().to_string();
        Self::is_valid_commit(&sha)?;

        Ok(Commit {
            full_hash: Self::get_full_hash(&sha)?,
            message: Self::get_message(&sha)?,
            diff: Self::get_diff(&sha)?,
            author_name: Self::get_author_name(&sha)?,
            author_email: Self::get_author_email(&sha)?,
            date: Self::get_date(&sha)?,
        })
    }

    /// Validate that a SHA or ref resolves to a commit object.
    pub fn is_valid_commit(sha: &str) -> Result<(), LumenError> {
        let sha = sha.trim();
        let output = Command::new("git").args(["cat-file", "-t", sha]).output()?;
        let output_str = String::from_utf8(output.stdout)?;

        if output_str.trim() == "commit" {
            return Ok(());
        }

        Err(CommitError::InvalidCommit(sha.to_string()).into())
    }

    /// Resolve the full commit hash for a ref.
    fn get_full_hash(sha: &str) -> Result<String, LumenError> {
        let output = Command::new("git").args(["rev-parse", sha]).output()?;

        let full_hash = String::from_utf8(output.stdout)?.trim_end().to_string();
        Ok(full_hash)
    }

    /// Get the commit diff content.
    fn get_diff(sha: &str) -> Result<String, LumenError> {
        let output = Command::new("git")
            .args([
                "diff-tree",
                "-p",
                "--root",
                "--binary",
                "--no-color",
                "--compact-summary",
                sha,
            ])
            .args(GIT_DIFF_EXCLUSIONS)
            .output()?;

        let diff = String::from_utf8(output.stdout)?;
        if diff.is_empty() {
            return Err(CommitError::EmptyDiff(sha.to_string()).into());
        }

        Ok(diff)
    }

    /// Get the commit message body.
    fn get_message(sha: &str) -> Result<String, LumenError> {
        let output = Command::new("git")
            .args(["log", "--format=%B", "-n", "1", sha])
            .output()?;

        let message = String::from_utf8(output.stdout)?
            .trim_end_matches('\n')
            .to_string();
        Ok(message)
    }

    /// Get the commit author name.
    fn get_author_name(sha: &str) -> Result<String, LumenError> {
        let output = Command::new("git")
            .args(["log", "--format=%an", "-n", "1", sha])
            .output()?;

        let name = String::from_utf8(output.stdout)?.trim_end().to_string();
        Ok(name)
    }

    /// Get the commit author email.
    fn get_author_email(sha: &str) -> Result<String, LumenError> {
        let output = Command::new("git")
            .args(["log", "--format=%ae", "-n", "1", sha])
            .output()?;

        let email = String::from_utf8(output.stdout)?.trim_end().to_string();
        Ok(email)
    }

    /// Get the commit timestamp formatted for display.
    fn get_date(sha: &str) -> Result<String, LumenError> {
        let output = Command::new("git")
            .args([
                "log",
                "--format=%cd",
                "--date=format:%Y-%m-%d %H:%M:%S",
                "-n",
                "1",
                sha,
            ])
            .output()?;

        let date = String::from_utf8(output.stdout)?.trim_end().to_string();
        Ok(date)
    }
}

#[cfg(test)]
mod tests {
    use super::Commit;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn cwd_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("failed to spawn git");
        assert!(status.success(), "git command failed: {:?}", args);
    }

    fn make_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let dir = env::temp_dir().join(format!("lumen-test-{}-{}", std::process::id(), nanos));
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    struct RepoGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        dir: PathBuf,
        original: PathBuf,
    }

    impl RepoGuard {
        fn new() -> Self {
            let lock = cwd_lock().lock().expect("failed to lock cwd");
            let original = env::current_dir().expect("failed to get cwd");
            let dir = make_temp_dir();

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

    #[test]
    fn is_valid_commit_should_accept_trailing_newline() {
        let _repo = RepoGuard::new();

        let result = Commit::is_valid_commit("HEAD\n");
        assert!(result.is_ok());
    }

    #[test]
    fn root_commit_diff_should_not_be_empty() {
        let _repo = RepoGuard::new();

        let commit = Commit::new("HEAD".to_string()).expect("root commit should load");
        assert!(!commit.diff.trim().is_empty());
    }
}
