//! The coding agent that answers review threads: a pi session in a Rex pane,
//! found and nudged through the `rex` command line.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

const AGENT_PROGRAM: &str = "pi";

pub struct RexSession {
    pub id: String,
    pub label: String,
}

pub struct PaneProgram {
    pub argv0: String,
    pub cwd: PathBuf,
}

#[derive(Clone, Default)]
pub struct PaneStatus {
    pub state: String,
    pub updated_at: String,
}

/// What Lumiere asks of Rex. Every call may fail when Rex is absent or the pane is gone.
pub trait Rex {
    fn sessions(&self) -> Vec<RexSession>;
    fn terminal_blocks(&self, session: &str) -> Vec<String>;
    fn foreground_program(&self, session: &str, block: &str) -> Option<PaneProgram>;
    fn program_status(&self, session: &str, block: &str) -> Option<PaneStatus>;
    /// Pastes `message` into the pane's prompt and submits it. `message` may span
    /// several lines and must hold no control character other than line breaks.
    fn submit_message(&self, session: &str, block: &str, message: &str) -> Result<(), String>;
}

pub struct RexCli;

impl RexCli {
    fn command(args: &[&str]) -> Command {
        let mut command = Command::new("rex");
        command
            .args(["--autostart=false", "--timeout", "3s"])
            .args(args)
            .stdin(Stdio::null());
        command
    }

    fn json(args: &[&str]) -> Option<Value> {
        let output = Self::command(args).arg("--json").output().ok()?;
        if !output.status.success() {
            return None;
        }
        serde_json::from_slice(&output.stdout).ok()
    }

    /// Runs `rex` with `input` on its standard input.
    fn run(args: &[&str], input: &str) -> Result<(), String> {
        let cannot_run = |error: std::io::Error| format!("cannot run rex: {}", error);
        let mut child = Self::command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(cannot_run)?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.as_bytes()).map_err(cannot_run)?;
        }
        let output = child.wait_with_output().map_err(cannot_run)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

impl Rex for RexCli {
    fn sessions(&self) -> Vec<RexSession> {
        let Some(json) = Self::json(&["ls"]) else {
            return Vec::new();
        };
        json["sessions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|session| RexSession {
                id: text(session, "session_id"),
                label: text(session, "label"),
            })
            .collect()
    }

    fn terminal_blocks(&self, session: &str) -> Vec<String> {
        let Some(json) = Self::json(&["block", "ls", "-s", session]) else {
            return Vec::new();
        };
        json["blocks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| text(block, "flavor").starts_with("com.superlogical.terminal"))
            .map(|block| text(block, "block_id"))
            .collect()
    }

    fn foreground_program(&self, session: &str, block: &str) -> Option<PaneProgram> {
        let json =
            Self::json(&["do", "com.superlogical.terminal.process", "-s", session, "-b", block])?;
        let foreground = json.get("foreground").filter(|program| program.is_object())?;
        Some(PaneProgram {
            argv0: text(foreground, "argv0"),
            cwd: PathBuf::from(text(foreground, "cwd")),
        })
    }

    fn program_status(&self, session: &str, block: &str) -> Option<PaneStatus> {
        let json = Self::json(&[
            "do",
            "com.superlogical.terminal.program_status",
            "-s",
            session,
            "-b",
            block,
        ])?;
        let records = json["records"].as_array()?;
        let record = records
            .iter()
            .find(|record| text(record, "app") == AGENT_PROGRAM)
            .or(records.first())?;
        Some(PaneStatus {
            state: text(record, "state"),
            updated_at: text(record, "updated_at"),
        })
    }

    fn submit_message(&self, session: &str, block: &str, message: &str) -> Result<(), String> {
        // A bracketed paste: the program receives the line breaks as text instead
        // of as the key that submits.
        let paste = format!("\x1b[200~{}\x1b[201~", message);
        Self::run(&["send", "-s", session, "-b", block], &paste)?;
        Self::run(&["send-key", "-s", session, "-b", block, "enter"], "")
    }
}

#[derive(Clone)]
pub struct AgentPane {
    pub session: String,
    pub session_label: String,
    pub block: String,
    pub cwd: PathBuf,
    pub status: PaneStatus,
}

impl AgentPane {
    pub fn picker_label(&self) -> String {
        let mut label = format!("{} · {}", AGENT_PROGRAM, self.cwd.display());
        if !self.session_label.is_empty() {
            label.push_str(&format!(" · {}", self.session_label));
        }
        if !self.status.state.is_empty() {
            label.push_str(&format!(" · {}", self.status.state));
        }
        label
    }
}

fn runs_agent(program: &PaneProgram) -> bool {
    program.argv0 == AGENT_PROGRAM
}

/// Whether the agent is the foreground program of the pane right now.
fn pane_runs_agent(rex: &dyn Rex, session: &str, block: &str) -> bool {
    rex.foreground_program(session, block)
        .is_some_and(|program| runs_agent(&program))
}

/// The pane `block`, when the agent is its foreground program.
fn agent_pane_at(rex: &dyn Rex, session: &RexSession, block: &str) -> Option<AgentPane> {
    let program = rex.foreground_program(&session.id, block).filter(runs_agent)?;
    Some(AgentPane {
        status: rex.program_status(&session.id, block).unwrap_or_default(),
        session: session.id.clone(),
        session_label: session.label.clone(),
        block: block.to_string(),
        cwd: program.cwd,
    })
}

/// Panes running the agent in one of `review_roots`, most recently active first.
fn find_agent_panes(rex: &dyn Rex, review_roots: &[PathBuf]) -> Vec<AgentPane> {
    let mut panes = Vec::new();
    for session in rex.sessions() {
        for block in rex.terminal_blocks(&session.id) {
            let pane = agent_pane_at(rex, &session, &block)
                .filter(|pane| review_roots.contains(&pane.cwd));
            panes.extend(pane);
        }
    }
    panes.sort_by(|a, b| b.status.updated_at.cmp(&a.status.updated_at));
    panes
}

/// How the agent calls this lumiere: by the path of the running executable, because
/// the `lumiere` on the agent's PATH may be missing or another version.
fn lumiere_command() -> String {
    match std::env::current_exe() {
        Ok(executable) => format!("'{}'", executable.display()),
        Err(_) => "lumiere".to_string(),
    }
}

/// The message pasted into the agent's prompt: how to answer, then `comments`, the
/// comments themselves under their threads, so that the agent has nothing to look up
/// before replying. Control characters are dropped: one of them could end the paste
/// early, and the rest of the text would then be read as keys.
fn message_for_agent(review_path: &Path, comments: &str) -> String {
    let lumiere = lumiere_command();
    let path = review_path.display();
    let message = format!(
        "Lumiere review comments. Answer each thread below with:\n\
         {lumiere} thread reply <thread-id> --repo '{path}' --body '<reply>'\n\
         Reply in the thread first, in a few sentences, saying whether you agree and what \
         you would change. Edit code only after I approve in that thread. \
         Earlier comments of a thread: {lumiere} thread list --repo '{path}'\n\n\
         {comments}"
    );
    message
        .chars()
        .filter(|character| *character == '\n' || !character.is_control())
        .collect()
}

/// The agent pane this review talks to, if any.
pub struct AgentLink<'r> {
    rex: &'r dyn Rex,
    pane: Option<AgentPane>,
    /// The repository or workspace folder under review, named in every message.
    review_path: PathBuf,
}

impl<'r> AgentLink<'r> {
    /// Attaches to `agent_block` in the current Rex session when given, otherwise to
    /// the most recently active agent pane in one of `review_roots`.
    pub fn attach(
        rex: &'r dyn Rex,
        agent_block: Option<&str>,
        current_session: Option<&str>,
        review_roots: &[PathBuf],
        review_path: PathBuf,
    ) -> Self {
        let pane = match (agent_block, current_session) {
            (Some(block), Some(session)) => {
                let current_session = RexSession {
                    id: session.to_string(),
                    label: String::new(),
                };
                agent_pane_at(rex, &current_session, block)
            }
            (Some(_), None) => None,
            (None, _) => find_agent_panes(rex, review_roots).into_iter().next(),
        };
        Self {
            rex,
            pane,
            review_path,
        }
    }

    pub fn is_attached(&self) -> bool {
        self.pane.is_some()
    }

    pub fn attached_block(&self) -> Option<&str> {
        self.pane.as_ref().map(|pane| pane.block.as_str())
    }

    pub fn candidates(&self, review_roots: &[PathBuf]) -> Vec<AgentPane> {
        find_agent_panes(self.rex, review_roots)
    }

    pub fn set_pane(&mut self, pane: Option<AgentPane>) {
        self.pane = pane;
    }

    /// Short text for the footer.
    pub fn label(&self) -> String {
        match &self.pane {
            Some(pane) if pane.status.state.is_empty() => AGENT_PROGRAM.to_string(),
            Some(pane) => format!("{} {}", AGENT_PROGRAM, pane.status.state),
            None => "no agent".to_string(),
        }
    }

    /// Reads the agent's state again. Detaches when the pane no longer runs the agent.
    pub fn refresh(&mut self) {
        let Some(pane) = &mut self.pane else {
            return;
        };
        if pane_runs_agent(self.rex, &pane.session, &pane.block) {
            pane.status = self
                .rex
                .program_status(&pane.session, &pane.block)
                .unwrap_or_default();
        } else {
            self.pane = None;
        }
    }

    /// Sends `comments` to the agent, with how to answer them. The pane is checked
    /// first, because the same text pasted into a shell would be run as commands;
    /// when it no longer runs the agent, nothing is sent and the link is dropped.
    pub fn send_comments(&mut self, comments: &str) -> Result<(), String> {
        let Some(pane) = &self.pane else {
            return Err("no agent attached".to_string());
        };
        if !pane_runs_agent(self.rex, &pane.session, &pane.block) {
            self.pane = None;
            return Err(format!("the pane no longer runs {}", AGENT_PROGRAM));
        }
        let message = message_for_agent(&self.review_path, comments);
        self.rex.submit_message(&pane.session, &pane.block, &message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakePane {
        session: &'static str,
        block: &'static str,
        argv0: &'static str,
        cwd: &'static str,
        updated_at: &'static str,
    }

    struct FakeRex {
        panes: RefCell<Vec<FakePane>>,
        typed: RefCell<Vec<(String, String)>>,
    }

    impl FakeRex {
        fn with(panes: Vec<FakePane>) -> Self {
            Self {
                panes: RefCell::new(panes),
                typed: RefCell::new(Vec::new()),
            }
        }
    }

    impl Rex for FakeRex {
        fn sessions(&self) -> Vec<RexSession> {
            let mut ids: Vec<&str> = self.panes.borrow().iter().map(|pane| pane.session).collect();
            ids.dedup();
            ids.into_iter()
                .map(|id| RexSession {
                    id: id.to_string(),
                    label: format!("label of {}", id),
                })
                .collect()
        }

        fn terminal_blocks(&self, session: &str) -> Vec<String> {
            self.panes
                .borrow()
                .iter()
                .filter(|pane| pane.session == session)
                .map(|pane| pane.block.to_string())
                .collect()
        }

        fn foreground_program(&self, session: &str, block: &str) -> Option<PaneProgram> {
            self.panes
                .borrow()
                .iter()
                .find(|pane| pane.session == session && pane.block == block)
                .map(|pane| PaneProgram {
                    argv0: pane.argv0.to_string(),
                    cwd: PathBuf::from(pane.cwd),
                })
        }

        fn program_status(&self, session: &str, block: &str) -> Option<PaneStatus> {
            self.panes
                .borrow()
                .iter()
                .find(|pane| pane.session == session && pane.block == block)
                .map(|pane| PaneStatus {
                    state: "idle".to_string(),
                    updated_at: pane.updated_at.to_string(),
                })
        }

        fn submit_message(&self, _session: &str, block: &str, message: &str) -> Result<(), String> {
            self.typed.borrow_mut().push((block.to_string(), message.to_string()));
            Ok(())
        }
    }

    fn pane(
        session: &'static str,
        block: &'static str,
        argv0: &'static str,
        cwd: &'static str,
        updated_at: &'static str,
    ) -> FakePane {
        FakePane {
            session,
            block,
            argv0,
            cwd,
            updated_at,
        }
    }

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/work/api"), PathBuf::from("/work")]
    }

    fn attach<'r>(rex: &'r FakeRex, agent_block: Option<&str>) -> AgentLink<'r> {
        AgentLink::attach(rex, agent_block, Some("s1"), &roots(), PathBuf::from("/work"))
    }

    #[test]
    fn test_attaches_to_the_latest_agent_pane_at_a_review_root() {
        let rex = FakeRex::with(vec![
            pane("s1", "older", "pi", "/work/api", "2026-11-18T10:00:00Z"),
            pane("s1", "shell", "zsh", "/work", "2026-11-18T12:00:00Z"),
            pane("s1", "subdirectory", "pi", "/work/api/src", "2026-11-18T12:00:00Z"),
            pane("s2", "elsewhere", "pi", "/other", "2026-11-18T12:00:00Z"),
            pane("s2", "latest", "pi", "/work", "2026-11-18T11:00:00Z"),
        ]);

        let link = attach(&rex, None);

        assert_eq!(link.attached_block(), Some("latest"));
        let candidates: Vec<String> = link
            .candidates(&roots())
            .into_iter()
            .map(|pane| pane.block)
            .collect();
        assert_eq!(candidates, vec!["latest", "older"]);
    }

    #[test]
    fn test_agent_block_is_used_without_searching_and_only_if_it_runs_the_agent() {
        let rex = FakeRex::with(vec![
            pane("s1", "given", "pi", "/somewhere/else", "2026-11-18T10:00:00Z"),
            pane("s1", "shell", "zsh", "/work", "2026-11-18T10:00:00Z"),
            pane("s1", "matching", "pi", "/work", "2026-11-18T12:00:00Z"),
        ]);

        assert_eq!(attach(&rex, Some("given")).attached_block(), Some("given"));
        assert_eq!(attach(&rex, Some("shell")).attached_block(), None);
    }

    #[test]
    fn test_comments_reach_the_attached_pane_with_the_reply_command_of_this_executable() {
        let rex = FakeRex::with(vec![pane("s1", "agent", "pi", "/work", "")]);
        let mut link = attach(&rex, None);
        let comments = "## Thread 1a2b · a.rs line 2 (RIGHT)\n\nyou: why?\x1b[201~";

        assert_eq!(link.send_comments(comments), Ok(()));

        let typed = rex.typed.borrow();
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].0, "agent");
        let this_executable = std::env::current_exe().expect("test executable");
        let reply_command = format!(
            "'{}' thread reply <thread-id> --repo '/work' --body",
            this_executable.display()
        );
        assert!(typed[0].1.contains(&reply_command), "{}", typed[0].1);
        // The comments come last and whole, minus the character that would end the paste.
        assert!(
            typed[0].1.ends_with("## Thread 1a2b · a.rs line 2 (RIGHT)\n\nyou: why?[201~"),
            "{}",
            typed[0].1
        );
    }

    #[test]
    fn test_nothing_is_typed_into_a_pane_that_stopped_running_the_agent() {
        let rex = FakeRex::with(vec![pane("s1", "agent", "pi", "/work", "")]);
        let mut link = attach(&rex, None);
        rex.panes.borrow_mut()[0].argv0 = "zsh";

        let error = link.send_comments("you: why?").expect_err("the agent quit");

        assert_eq!(error, "the pane no longer runs pi");
        assert!(rex.typed.borrow().is_empty());
        assert!(!link.is_attached());
        assert_eq!(link.label(), "no agent");
    }
}
