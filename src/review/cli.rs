//! `lumiere thread`: CLI interface used by coding agents to inspect and reply to review threads.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::store::ThreadStore;
use super::thread::{AnnotationTarget, Comment, Thread};
use super::workspace::Workspace;
use crate::vcs::{self, VcsBackendType};

pub struct ReviewRepository {
    name: String,
    root: PathBuf,
}

pub fn repositories_at(
    path: &Path,
    vcs_override: Option<VcsBackendType>,
) -> Result<Vec<ReviewRepository>, String> {
    if let Some(workspace) = Workspace::discover(path, vcs_override) {
        return Ok(workspace
            .repository_roots()
            .into_iter()
            .map(|(name, root)| ReviewRepository {
                name,
                root: without_trailing_slash(&root),
            })
            .collect());
    }
    let backend = vcs::get_backend(path, vcs_override).map_err(|error| error.to_string())?;
    let root = without_trailing_slash(backend.working_tree_root());
    let name = root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    Ok(vec![ReviewRepository { name, root }])
}

fn without_trailing_slash(path: &Path) -> PathBuf {
    path.components().collect()
}

fn visible_threads(repository: &ReviewRepository, awaiting: bool) -> Result<Vec<Thread>, String> {
    let threads = ThreadStore::for_repository(&repository.root)?.load()?;
    Ok(threads
        .into_iter()
        .filter(|thread| thread.sent_comments().next().is_some())
        .filter(|thread| !awaiting || thread.awaiting_agent())
        .collect())
}

fn thread_json(repository: &ReviewRepository, thread: &Thread) -> Value {
    let (side, start_line, end_line) = match &thread.target {
        AnnotationTarget::File => (Value::Null, Value::Null, Value::Null),
        AnnotationTarget::LineRange {
            panel,
            start_line,
            end_line,
        } => (json!(panel), json!(start_line), json!(end_line)),
    };
    json!({
        "id": thread.id,
        "repository": repository.name,
        "repository_path": repository.root,
        "file": thread.filename,
        "side": side,
        "start_line": start_line,
        "end_line": end_line,
        "anchored_code": thread.anchor_text,
        "resolved": thread.resolved,
        "outdated": thread.outdated,
        "awaiting_agent": thread.awaiting_agent(),
        "comments": thread
            .sent_comments()
            .map(|comment| json!({
                "author": comment.author,
                "author_name": comment.author_name,
                "body": comment.body,
                "created_at": comment.created_at,
            }))
            .collect::<Vec<_>>(),
    })
}

fn repository_heading(repository: &ReviewRepository) -> String {
    format!(
        "# Repository: {} ({})",
        repository.name,
        repository.root.display()
    )
}

fn no_such_thread(thread_id: &str) -> String {
    format!("no thread {thread_id} here; run `lumiere thread list` to see the thread ids")
}

pub fn show(
    repositories: &[ReviewRepository],
    thread_id: &str,
    json: bool,
) -> Result<String, String> {
    for repository in repositories {
        let threads = visible_threads(repository, false)?;
        let Some(thread) = threads.iter().find(|thread| thread.id == thread_id) else {
            continue;
        };
        if json {
            return serde_json::to_string_pretty(&thread_json(repository, thread))
                .map_err(|error| error.to_string());
        }
        return Ok(format!(
            "{}\n\n{}",
            repository_heading(repository),
            thread.describe(thread.sent_comments())
        ));
    }
    Err(no_such_thread(thread_id))
}

pub fn list(
    repositories: &[ReviewRepository],
    awaiting: bool,
    json: bool,
) -> Result<String, String> {
    let mut json_threads = Vec::new();
    let mut sections = Vec::new();
    for repository in repositories {
        let threads = visible_threads(repository, awaiting)?;
        if threads.is_empty() {
            continue;
        }
        json_threads.extend(threads.iter().map(|thread| thread_json(repository, thread)));
        let mut section = repository_heading(repository);
        for thread in &threads {
            section.push_str(&format!("\n\n{}", thread.describe(thread.sent_comments())));
        }
        sections.push(section);
    }

    if json {
        return serde_json::to_string_pretty(&json_threads).map_err(|error| error.to_string());
    }
    if sections.is_empty() {
        return Ok(if awaiting {
            "No thread is awaiting a reply.".to_string()
        } else {
            "No threads.".to_string()
        });
    }
    Ok(sections.join("\n\n"))
}

pub fn reply(
    repositories: &[ReviewRepository],
    thread_id: &str,
    body: &str,
    author_name: &str,
) -> Result<String, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("the reply is empty".to_string());
    }

    for repository in repositories {
        let store = ThreadStore::for_repository(&repository.root)?;
        let (_, outcome) = store.update(|threads| {
            let thread = threads
                .iter_mut()
                .find(|t| t.id == thread_id && t.sent_comments().next().is_some())?;
            if thread.resolved {
                return Some(Err(format!("thread {thread_id} is resolved")));
            }
            thread.add_agent_reply(Comment::agent(body.to_string(), author_name.to_string()));
            Some(Ok(format!(
                "Replied to thread {thread_id} on {}.",
                thread.filename
            )))
        })?;
        if let Some(outcome) = outcome {
            return outcome;
        }
    }
    Err(no_such_thread(thread_id))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::diff::DiffPanelFocus;
    use crate::vcs::test_utils::git;

    fn create_repository(parent: &Path, name: &str) -> PathBuf {
        let dir = parent.join(name);
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init"]);
        dir
    }

    fn store_thread(repository_root: &Path, body: &str, queued: bool) -> String {
        let thread = Thread::new(
            "src/api.rs".to_string(),
            AnnotationTarget::LineRange {
                panel: DiffPanelFocus::New,
                start_line: 2,
                end_line: 2,
            },
            Some("    run();".to_string()),
            Comment::human(body.to_string(), queued),
        );
        let thread_id = thread.id.clone();
        ThreadStore::for_repository(repository_root)
            .unwrap()
            .update(|threads| threads.push(thread.clone()))
            .unwrap();
        thread_id
    }

    fn stored_threads(repository_root: &Path) -> Vec<Thread> {
        ThreadStore::for_repository(repository_root)
            .unwrap()
            .load()
            .unwrap()
    }

    #[test]
    fn test_list_shows_what_the_agent_needs_and_hides_unsent_comments() {
        let parent = TempDir::new().unwrap();
        let root = create_repository(parent.path(), "api");
        let thread_id = store_thread(&root, "why not async?", false);
        store_thread(&root, "not sent yet", true);
        let repositories = repositories_at(&root, None).unwrap();

        let text = list(&repositories, true, false).unwrap();
        let json: Value = serde_json::from_str(&list(&repositories, false, true).unwrap()).unwrap();

        assert_eq!(
            text,
            format!(
                "# Repository: api ({})\n\n\
                 ## Thread {} · src/api.rs line 2 (RIGHT) · awaiting your reply\n\n\
                 ```\n    run();\n```\n\n\
                 you: why not async?",
                repositories[0].root.display(),
                thread_id
            )
        );
        assert_eq!(json.as_array().map(Vec::len), Some(1));
        assert_eq!(json[0]["id"], thread_id.as_str());
        assert_eq!(json[0]["side"], "new");
        assert_eq!(json[0]["start_line"], 2);
        assert_eq!(json[0]["anchored_code"], "    run();");
        assert_eq!(json[0]["awaiting_agent"], true);
        assert_eq!(json[0]["comments"][0]["body"], "why not async?");
    }

    #[test]
    fn test_show_and_reply_across_workspace_repositories() {
        let parent = TempDir::new().unwrap();
        let api = create_repository(parent.path(), "api");
        let web = create_repository(parent.path(), "web");
        store_thread(&api, "api comment", false);
        let thread_id = store_thread(&web, "why not async?", false);
        let unsent_id = store_thread(&web, "not sent yet", true);
        let repositories = repositories_at(parent.path(), None).unwrap();

        let message = reply(
            &repositories,
            &thread_id,
            "  It blocks nothing here.\n",
            "pi",
        )
        .unwrap();
        assert_eq!(
            message,
            format!("Replied to thread {thread_id} on src/api.rs.")
        );

        let text = show(&repositories, &thread_id, false).unwrap();
        let json: Value =
            serde_json::from_str(&show(&repositories, &thread_id, true).unwrap()).unwrap();
        assert!(text.contains("you: why not async?\n\npi: It blocks nothing here."));
        assert_eq!(json["repository"], "web");
        assert_eq!(json["comments"].as_array().map(Vec::len), Some(2));
        assert_eq!(stored_threads(&api)[0].comments.len(), 1);
        assert_eq!(stored_threads(&web)[0].comments.len(), 2);

        assert!(show(&repositories, "nope", false)
            .unwrap_err()
            .starts_with("no thread nope here"));
        assert!(show(&repositories, &unsent_id, false).is_err());

        let list_text = list(&repositories, true, false).unwrap();
        assert!(!list_text.contains("/)"));
        assert!(list_text.contains("# Repository: api ("));
        assert!(!list_text.contains("# Repository: web ("));
    }

    #[test]
    fn test_reply_refused_for_unknown_unsent_resolved_and_empty() {
        let parent = TempDir::new().unwrap();
        let root = create_repository(parent.path(), "api");
        let unsent_id = store_thread(&root, "not sent yet", true);
        let resolved_id = store_thread(&root, "done", false);
        ThreadStore::for_repository(&root)
            .unwrap()
            .update(|threads| threads[1].resolved = true)
            .unwrap();
        let repositories = repositories_at(&root, None).unwrap();

        assert!(reply(&repositories, "nope", "text", "pi").is_err());
        assert!(reply(&repositories, &unsent_id, "text", "pi").is_err());
        assert_eq!(
            reply(&repositories, &resolved_id, "text", "pi").unwrap_err(),
            format!("thread {resolved_id} is resolved")
        );
        assert_eq!(
            reply(&repositories, &resolved_id, " \n", "pi").unwrap_err(),
            "the reply is empty"
        );
        assert!(stored_threads(&root).iter().all(|t| t.comments.len() == 1));
    }
}
