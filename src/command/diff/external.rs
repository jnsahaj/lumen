use std::fs;
use std::io::Read;

use super::types::{is_binary_content, FileDiff, FileStatus};

const DEV_NULL: &str = "/dev/null";

pub fn load_stdin_diffs() -> Result<Vec<FileDiff>, String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("Failed to read diff from stdin: {}", e))?;

    if input.trim().is_empty() {
        return Err("No diff received on stdin".to_string());
    }

    let diffs = parse_unified_diff(&input);
    if diffs.is_empty() {
        return Err("Could not parse any file diffs from stdin input".to_string());
    }

    reconnect_stdin_to_tty();

    Ok(diffs)
}

#[cfg(unix)]
fn reconnect_stdin_to_tty() {
    use std::os::unix::io::AsRawFd;

    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        unsafe {
            if libc::isatty(fd) == 1 {
                libc::dup2(fd, libc::STDIN_FILENO);
                return;
            }
        }
    }

    if let Ok(tty) = fs::OpenOptions::new().read(true).write(true).open("/dev/tty") {
        unsafe {
            libc::dup2(tty.as_raw_fd(), libc::STDIN_FILENO);
        }
    }
}

#[cfg(not(unix))]
fn reconnect_stdin_to_tty() {}

pub fn load_files_diffs(paths: &[String]) -> Result<Vec<FileDiff>, String> {
    match paths.len() {
        2 => {
            let old_content = read_file_or_empty(&paths[0])?;
            let new_content = read_file_or_empty(&paths[1])?;

            let status = if old_content.is_empty() && !new_content.is_empty() {
                FileStatus::Added
            } else if !old_content.is_empty() && new_content.is_empty() {
                FileStatus::Deleted
            } else {
                FileStatus::Modified
            };

            let is_binary = is_binary_content(&old_content) || is_binary_content(&new_content);

            Ok(vec![FileDiff {
                filename: paths[1].clone(),
                old_content,
                new_content,
                status,
                is_binary,
            }])
        }
        3 => Err(
            "Three-file (diff3/merge) mode is not supported yet. Use two files or --stdin."
                .to_string(),
        ),
        n => Err(format!(
            "--files expects 2 paths (old new), got {}. Three-way merge is not supported yet.",
            n
        )),
    }
}

fn read_file_or_empty(path: &str) -> Result<String, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("Failed to read {}: {}", path, e)),
    }
}

struct FileAcc {
    old_path: String,
    new_path: String,
    old_content: String,
    new_content: String,
    is_binary: bool,
    saw_hunk: bool,
}

impl FileAcc {
    fn new(old_path: String, new_path: String) -> Self {
        Self {
            old_path,
            new_path,
            old_content: String::new(),
            new_content: String::new(),
            is_binary: false,
            saw_hunk: false,
        }
    }

    fn is_pristine(&self) -> bool {
        !self.saw_hunk
            && !self.is_binary
            && self.old_content.is_empty()
            && self.new_content.is_empty()
    }

    fn build(self) -> FileDiff {
        let filename = if self.new_path == DEV_NULL {
            self.old_path.clone()
        } else {
            self.new_path.clone()
        };

        let status = if self.old_path == DEV_NULL {
            FileStatus::Added
        } else if self.new_path == DEV_NULL {
            FileStatus::Deleted
        } else {
            FileStatus::Modified
        };

        let is_binary = self.is_binary
            || is_binary_content(&self.old_content)
            || is_binary_content(&self.new_content);

        FileDiff {
            filename,
            old_content: self.old_content,
            new_content: self.new_content,
            status,
            is_binary,
        }
    }
}

pub fn parse_unified_diff(input: &str) -> Vec<FileDiff> {
    let lines: Vec<&str> = input.lines().collect();
    let mut files: Vec<FileDiff> = Vec::new();
    let mut cur: Option<FileAcc> = None;

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        if let Some((a, b)) = parse_diff_git_header(line) {
            if let Some(acc) = cur.take() {
                files.push(acc.build());
            }
            cur = Some(FileAcc::new(a, b));
            i += 1;
            continue;
        }

        if let Some(rest) = line.strip_prefix("--- ") {
            if i + 1 < lines.len() {
                if let Some(new_rest) = lines[i + 1].strip_prefix("+++ ") {
                    let old_path = parse_header_path(rest);
                    let new_path = parse_header_path(new_rest);
                    match cur.as_mut() {
                        Some(acc) if acc.is_pristine() => {
                            acc.old_path = old_path;
                            acc.new_path = new_path;
                        }
                        _ => {
                            if let Some(acc) = cur.take() {
                                files.push(acc.build());
                            }
                            cur = Some(FileAcc::new(old_path, new_path));
                        }
                    }
                    i += 2;
                    continue;
                }
            }
        }

        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            if let Some(acc) = cur.as_mut() {
                acc.is_binary = true;
            }
            i += 1;
            continue;
        }

        if line.starts_with("@@") {
            if let Some(acc) = cur.as_mut() {
                acc.saw_hunk = true;
            }
            i += 1;
            continue;
        }

        if let Some(acc) = cur.as_mut() {
            if acc.saw_hunk {
                match line.as_bytes().first() {
                    Some(b' ') => {
                        push_line(&mut acc.old_content, &line[1..]);
                        push_line(&mut acc.new_content, &line[1..]);
                    }
                    Some(b'-') => push_line(&mut acc.old_content, &line[1..]),
                    Some(b'+') => push_line(&mut acc.new_content, &line[1..]),
                    Some(b'\\') => {}
                    None => {
                        acc.old_content.push('\n');
                        acc.new_content.push('\n');
                    }
                    _ => {}
                }
            }
        }

        i += 1;
    }

    if let Some(acc) = cur.take() {
        files.push(acc.build());
    }

    files
}

fn push_line(buf: &mut String, content: &str) {
    buf.push_str(content);
    buf.push('\n');
}

fn parse_diff_git_header(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?;
    if let Some(idx) = rest.rfind(" b/") {
        let a = &rest[..idx];
        let b = &rest[idx + 1..];
        return Some((strip_ab_prefix(a), strip_ab_prefix(b)));
    }
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() >= 2 {
        return Some((
            strip_ab_prefix(parts[0]),
            strip_ab_prefix(parts[parts.len() - 1]),
        ));
    }
    None
}

fn parse_header_path(raw: &str) -> String {
    let trimmed = raw.split('\t').next().unwrap_or(raw).trim_end();
    if trimmed == DEV_NULL {
        return DEV_NULL.to_string();
    }
    strip_ab_prefix(trimmed)
}

fn strip_ab_prefix(path: &str) -> String {
    let p = path.trim();
    p.strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .unwrap_or(p)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_modified_file() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
 }
";
        let diffs = parse_unified_diff(diff);
        assert_eq!(diffs.len(), 1);
        let d = &diffs[0];
        assert_eq!(d.filename, "src/main.rs");
        assert_eq!(d.status, FileStatus::Modified);
        assert_eq!(d.old_content, "fn main() {\n    println!(\"old\");\n}\n");
        assert_eq!(d.new_content, "fn main() {\n    println!(\"new\");\n}\n");
    }

    #[test]
    fn test_parse_added_file() {
        let diff = "\
diff --git a/new.txt b/new.txt
new file mode 100644
index 000..222
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+line one
+line two
";
        let diffs = parse_unified_diff(diff);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].filename, "new.txt");
        assert_eq!(diffs[0].status, FileStatus::Added);
        assert!(diffs[0].old_content.is_empty());
        assert_eq!(diffs[0].new_content, "line one\nline two\n");
    }

    #[test]
    fn test_parse_deleted_file() {
        let diff = "\
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-bye one
-bye two
";
        let diffs = parse_unified_diff(diff);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].filename, "gone.txt");
        assert_eq!(diffs[0].status, FileStatus::Deleted);
        assert_eq!(diffs[0].old_content, "bye one\nbye two\n");
        assert!(diffs[0].new_content.is_empty());
    }

    #[test]
    fn test_parse_multiple_files() {
        let diff = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-a old
+a new
diff --git a/b.txt b/b.txt
--- a/b.txt
+++ b/b.txt
@@ -1 +1 @@
-b old
+b new
";
        let diffs = parse_unified_diff(diff);
        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].filename, "a.txt");
        assert_eq!(diffs[1].filename, "b.txt");
        assert_eq!(diffs[1].new_content, "b new\n");
    }

    #[test]
    fn test_parse_plain_unified_diff_no_git_header() {
        let diff = "\
--- old.txt\t2020-01-01
+++ new.txt\t2020-01-02
@@ -1,2 +1,2 @@
 keep
-remove
+add
";
        let diffs = parse_unified_diff(diff);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].filename, "new.txt");
        assert_eq!(diffs[0].old_content, "keep\nremove\n");
        assert_eq!(diffs[0].new_content, "keep\nadd\n");
    }

    #[test]
    fn test_parse_multiple_hunks() {
        let diff = "\
diff --git a/f.txt b/f.txt
--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,2 @@
 first
-second
+SECOND
@@ -10,2 +10,2 @@
 tenth
-eleventh
+ELEVENTH
";
        let diffs = parse_unified_diff(diff);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].old_content, "first\nsecond\ntenth\neleventh\n");
        assert_eq!(diffs[0].new_content, "first\nSECOND\ntenth\nELEVENTH\n");
    }

    #[test]
    fn test_parse_binary_file() {
        let diff = "\
diff --git a/img.png b/img.png
index 111..222 100644
Binary files a/img.png and b/img.png differ
";
        let diffs = parse_unified_diff(diff);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].filename, "img.png");
        assert!(diffs[0].is_binary);
    }

    #[test]
    fn test_parse_empty() {
        assert!(parse_unified_diff("").is_empty());
        assert!(parse_unified_diff("not a diff at all\njust text\n").is_empty());
    }

    #[test]
    fn test_load_files_diffs_three_files_errors() {
        let paths = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert!(load_files_diffs(&paths).is_err());
    }
}
