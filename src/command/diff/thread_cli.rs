//! `lumiere thread`: how a coding agent reads review threads and answers them.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::review_thread::{AnnotationTarget, Comment, Thread};
use super::thread_store::ThreadStore;
use super::workspace::Workspace;
use crate::vcs::{self, VcsBackendType};

pub struct ReviewRepository {
    name: String,
    root: PathBuf,
}

/// The repository containing `path`, or every repository directly below it when
/// `path` is a workspace folder.
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
    let name = root
        .file_name()
        .map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned());
    Ok(vec![ReviewRepository { name, root }])
}

/// Git reports a working tree root that ends with a slash.
fn without_trailing_slash(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Threads the agent can see: those with at least one comment sent to it.
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
    format!("# Repository: {} ({})", repository.name, repository.root.display())
}

fn no_such_thread(thread_id: &str) -> String {
    format!(
        "no thread {} here; run `lumiere thread list` to see the thread ids",
        thread_id
    )
}

/// One thread of `repositories`, with its code and every comment sent to the agent,
/// as text for a reader or as JSON.
pub fn show(repositories: &[ReviewRepository], thread_id: &str, json: bool) -> Result<String, String> {
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

/// The threads of `repositories`, as text for a reader or as JSON.
pub fn list(repositories: &[ReviewRepository], awaiting: bool, json: bool) -> Result<String, String> {
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

/// Adds the agent's reply to a thread of one of `repositories`.
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
                .find(|thread| thread.id == thread_id && thread.sent_comments().next().is_some())?;
            if thread.resolved {
                return Some(Err(format!("thread {} is resolved", thread_id)));
            }
            thread.add_agent_reply(Comment::agent(body.to_string(), author_name.to_string()));
            Some(Ok(format!("Replied to thread {} on {}.", thread_id, thread.filename)))
        })?;
        if let Some(outcome) = outcome {
            return outcome;
        }
    }
    Err(no_such_thread(thread_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::diff::types::DiffPanelFocus;
    use crate::vcs::test_utils::{git, make_temp_dir};
    use std::fs;

    fn create_repository(parent: &Path, name: &str) -> PathBuf {
        let dir = parent.join(name);
        fs::create_dir_all(&dir).expect("create repository dir");
        git(&dir, &["init"]);
        dir
    }

    /// Stores a thread on line 2 of `src/api.rs` and returns its id.
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
            .expect("store")
            .update(|threads| threads.push(thread.clone()))
            .expect("store thread");
        thread_id
    }

    fn stored_threads(repository_root: &Path) -> Vec<Thread> {
        ThreadStore::for_repository(repository_root)
            .expect("store")
            .load()
            .expect("load")
    }

    #[test]
    fn test_list_shows_what_the_agent_needs_and_hides_unsent_comments() {
        let parent = make_temp_dir("thread-cli-list");
        let root = create_repository(&parent, "api");
        let thread_id = store_thread(&root, "why not async?", false);
        store_thread(&root, "not sent yet", true);
        let repositories = repositories_at(&root, None).expect("repository");

        let text = list(&repositories, true, false).expect("list");
        let json: Value =
            serde_json::from_str(&list(&repositories, false, true).expect("json")).expect("parse");

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
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn test_show_gives_one_thread_with_its_code_and_every_sent_comment() {
        let parent = make_temp_dir("thread-cli-show");
        let api = create_repository(&parent, "api");
        let web = create_repository(&parent, "web");
        store_thread(&api, "another thread", false);
        let thread_id = store_thread(&web, "why not async?", false);
        let unsent_id = store_thread(&web, "not sent yet", true);
        let repositories = repositories_at(&parent, None).expect("workspace");
        reply(&repositories, &thread_id, "It blocks nothing here.", "pi").expect("reply");

        let text = show(&repositories, &thread_id, false).expect("show");
        let json: Value =
            serde_json::from_str(&show(&repositories, &thread_id, true).expect("json")).expect("parse");

        assert_eq!(
            text,
            format!(
                "# Repository: web ({})\n\n\
                 ## Thread {} · src/api.rs line 2 (RIGHT)\n\n\
                 ```\n    run();\n```\n\n\
                 you: why not async?\n\n\
                 pi: It blocks nothing here.",
                repositories[1].root.display(),
                thread_id
            )
        );
        assert_eq!(json["id"], thread_id.as_str());
        assert_eq!(json["repository"], "web");
        assert_eq!(json["comments"].as_array().map(Vec::len), Some(2));

        let unknown = show(&repositories, "nope", false).expect_err("unknown");
        let unsent = show(&repositories, &unsent_id, false).expect_err("unsent");
        assert!(unknown.starts_with("no thread nope here"), "{}", unknown);
        assert!(unsent.starts_with(&format!("no thread {} here", unsent_id)), "{}", unsent);
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn test_reply_is_stored_and_the_thread_stops_awaiting() {
        let parent = make_temp_dir("thread-cli-reply");
        let root = create_repository(&parent, "api");
        let thread_id = store_thread(&root, "why not async?", false);
        let repositories = repositories_at(&root, None).expect("repository");

        let message = reply(&repositories, &thread_id, "  It blocks nothing here.\n", "pi").expect("reply");

        assert_eq!(message, format!("Replied to thread {} on src/api.rs.", thread_id));
        let threads = stored_threads(&root);
        assert_eq!(threads[0].comments[1].body, "It blocks nothing here.");
        assert_eq!(threads[0].comments[1].author_name, "pi");
        assert_eq!(
            list(&repositories, true, false).expect("list"),
            "No thread is awaiting a reply."
        );
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn test_reply_is_refused_for_unknown_unsent_and_resolved_threads() {
        let parent = make_temp_dir("thread-cli-refused");
        let root = create_repository(&parent, "api");
        let unsent_id = store_thread(&root, "not sent yet", true);
        let resolved_id = store_thread(&root, "done", false);
        ThreadStore::for_repository(&root)
            .expect("store")
            .update(|threads| threads[1].resolved = true)
            .expect("resolve");
        let repositories = repositories_at(&root, None).expect("repository");

        let unknown = reply(&repositories, "nope", "text", "pi").expect_err("unknown");
        let unsent = reply(&repositories, &unsent_id, "text", "pi").expect_err("unsent");
        let resolved = reply(&repositories, &resolved_id, "text", "pi").expect_err("resolved");
        let empty = reply(&repositories, &resolved_id, " \n", "pi").expect_err("empty");

        assert!(unknown.starts_with("no thread nope here"), "{}", unknown);
        assert!(unsent.starts_with(&format!("no thread {} here", unsent_id)), "{}", unsent);
        assert_eq!(resolved, format!("thread {} is resolved", resolved_id));
        assert_eq!(empty, "the reply is empty");
        assert!(stored_threads(&root).iter().all(|thread| thread.comments.len() == 1));
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn test_workspace_folder_covers_every_repository() {
        let parent = make_temp_dir("thread-cli-workspace");
        let api = create_repository(&parent, "api");
        let web = create_repository(&parent, "web");
        store_thread(&api, "api comment", false);
        let web_thread_id = store_thread(&web, "web comment", false);
        let repositories = repositories_at(&parent, None).expect("workspace");

        let text = list(&repositories, true, false).expect("list");
        reply(&repositories, &web_thread_id, "answer", "pi").expect("reply");

        assert!(!text.contains("/)"), "{}", text);
        let api_heading = text.find("# Repository: api (").expect("api section");
        let web_heading = text.find("# Repository: web (").expect("web section");
        assert!(api_heading < web_heading);
        assert_eq!(stored_threads(&web)[0].comments.len(), 2);
        assert_eq!(stored_threads(&api)[0].comments.len(), 1);
        let _ = fs::remove_dir_all(&parent);
    }
}
