//! The coding agent that answers review threads: a session of pi, Claude Code or
//! another agent in a Rex pane, found and nudged through the `rex` command line.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use super::review_thread::EARLIER_COMMENTS_LABEL;

/// The names coding agents give themselves in the status they report to Rex. Other
/// programs report a status too, a build tool for one, so the name decides.
const KNOWN_AGENTS: &[&str] = &[
    "pi",
    "claude-code",
    "antigravity",
    "codex",
    "opencode",
    "hermes",
    "grok",
    "grok-build",
];

/// The state of an agent that is asking its user something. A message submitted
/// then would answer that question.
const BLOCKED: &str = "blocked";

pub struct RexSession {
    pub id: String,
    pub label: String,
}

/// What a program in a pane reported about itself.
#[derive(Clone, Default)]
pub struct PaneStatus {
    pub app: String,
    pub state: String,
    pub updated_at: String,
    /// The pane's foreground process when the report arrived.
    pub owner_pid: Option<u64>,
}

/// A terminal pane, as Rex describes it.
pub struct RexPane {
    pub block: String,
    /// Where the foreground program runs, or the pane's first program when the
    /// foreground one belongs to another user and cannot be read.
    pub cwd: PathBuf,
    pub foreground_pid: Option<u64>,
    pub statuses: Vec<PaneStatus>,
}

/// What Lumiere asks of Rex. Every call may fail when Rex is absent or the pane is gone.
pub trait Rex {
    fn sessions(&self) -> Vec<RexSession>;
    fn terminal_panes(&self, session: &str) -> Vec<RexPane>;
    fn terminal_pane(&self, session: &str, block: &str) -> Option<RexPane>;
    /// Pastes `message` into the pane's prompt and submits it. `message` may span
    /// several lines and must hold no control character other than line breaks.
    fn submit_message(&self, session: &str, block: &str, message: &str) -> Result<(), String>;
}

/// Describes the terminal panes of the session `rex do` runs in, or only the pane
/// it runs in when `only_block` is given: one `rex` process instead of two per pane.
const PANES_SCRIPT: &str = r#"
local session = rex.session_id
local blocks
if rex.args.only_block then
  blocks = { { block_id = rex.block_id, creator_name = "com.superlogical.terminal" } }
else
  blocks = (rex.call("session.list_blocks", { session_id = session }) or {}).blocks or {}
end
local panes = {}
for _, block in ipairs(blocks) do
  if block.creator_name == "com.superlogical.terminal" then
    local function ask(method)
      local ok, answer = pcall(rex.call, "block.method",
        { session_id = session, block_id = block.block_id, method = method })
      return ok and answer or nil
    end
    panes[#panes + 1] =
      { block = block.block_id, process = ask("process"), status = ask("program_status") }
  end
end
return panes
"#;

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

    fn panes(session: &str, only_block: Option<&str>) -> Vec<RexPane> {
        let mut args = vec!["do", "-s", session];
        if let Some(block) = only_block {
            args.extend(["-b", block, "--args", r#"{"only_block":true}"#]);
        }
        args.extend(["-e", PANES_SCRIPT]);
        Self::json(&args).map(|panes| panes_from(&panes)).unwrap_or_default()
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

/// Reads what `PANES_SCRIPT` returned. An empty list arrives as an empty object.
fn panes_from(panes: &Value) -> Vec<RexPane> {
    let pane_from = |pane: &Value| {
        let foreground = &pane["process"]["foreground"];
        let first_program = &pane["process"]["child"];
        let cwd = [foreground, first_program]
            .into_iter()
            .map(|program| text(program, "cwd"))
            .find(|cwd| !cwd.is_empty())
            .unwrap_or_default();
        let status_from = |record: &Value| PaneStatus {
            app: text(record, "app"),
            state: text(record, "state"),
            updated_at: text(record, "updated_at"),
            owner_pid: record["owner"]["pid"].as_u64(),
        };
        let records = pane["status"]["records"].as_array();
        RexPane {
            block: text(pane, "block"),
            cwd: PathBuf::from(cwd),
            foreground_pid: foreground["pid"].as_u64(),
            statuses: records.into_iter().flatten().map(status_from).collect(),
        }
    };
    panes.as_array().into_iter().flatten().map(pane_from).collect()
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

    fn terminal_panes(&self, session: &str) -> Vec<RexPane> {
        Self::panes(session, None)
    }

    fn terminal_pane(&self, session: &str, block: &str) -> Option<RexPane> {
        Self::panes(session, Some(block)).into_iter().next()
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
    /// What the agent reported, with the name it gives itself.
    pub status: PaneStatus,
}

impl AgentPane {
    pub fn picker_label(&self) -> String {
        let mut label = format!("{} · {}", self.status.app, self.cwd.display());
        if !self.session_label.is_empty() {
            label.push_str(&format!(" · {}", self.session_label));
        }
        if !self.status.state.is_empty() {
            label.push_str(&format!(" · {}", self.status.state));
        }
        label
    }
}

/// The status of the agent running in `pane`, if one is. Rex keeps the last status
/// of a program that has exited, so only one reported by the program now in the
/// foreground counts: a message for the agent must not reach the shell behind it.
fn running_agent(pane: &RexPane) -> Option<&PaneStatus> {
    pane.statuses.iter().find(|status| {
        KNOWN_AGENTS.contains(&status.app.as_str())
            && status.owner_pid.is_some()
            && status.owner_pid == pane.foreground_pid
    })
}

/// `pane` as an agent pane, when an agent runs in it.
fn agent_pane(session: &RexSession, pane: RexPane) -> Option<AgentPane> {
    let status = running_agent(&pane)?.clone();
    Some(AgentPane {
        session: session.id.clone(),
        session_label: session.label.clone(),
        block: pane.block,
        cwd: pane.cwd,
        status,
    })
}

/// Panes running an agent in one of `review_roots` or in a folder inside one, most
/// recently active first.
fn find_agent_panes(rex: &dyn Rex, review_roots: &[PathBuf]) -> Vec<AgentPane> {
    let mut panes = Vec::new();
    for session in rex.sessions() {
        for pane in rex.terminal_panes(&session.id) {
            let pane = agent_pane(&session, pane)
                .filter(|pane| review_roots.iter().any(|root| pane.cwd.starts_with(root)));
            panes.extend(pane);
        }
    }
    panes.sort_by(|a, b| b.status.updated_at.cmp(&a.status.updated_at));
    panes
}

const PROGRAM_NAME: &str = "lumiere";

/// How the agent calls this lumiere: `lumiere` when that name on PATH is the running
/// executable, else the executable's path, because the `lumiere` on the agent's PATH
/// may be missing or another version.
fn lumiere_command() -> String {
    match std::env::current_exe() {
        Ok(executable) => command_for(&executable, std::env::var_os("PATH").as_deref()),
        Err(_) => PROGRAM_NAME.to_string(),
    }
}

fn command_for(executable: &Path, path_variable: Option<&std::ffi::OsStr>) -> String {
    let first_on_path = path_variable
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|directory| directory.join(PROGRAM_NAME))
        .find(|candidate| candidate.is_file());
    let is_the_executable = |candidate: &PathBuf| {
        matches!(
            (candidate.canonicalize(), executable.canonicalize()),
            (Ok(candidate), Ok(executable)) if candidate == executable
        )
    };
    if first_on_path.as_ref().is_some_and(is_the_executable) {
        PROGRAM_NAME.to_string()
    } else {
        format!("'{}'", executable.display())
    }
}

/// The message pasted into the agent's prompt: how to answer, then `comments`, the
/// comments themselves under their threads. A thread's first comment comes with its
/// code, so that the agent has nothing to look up before replying. A later comment
/// comes without it, and the message then says how to read the thread.
/// Control characters are dropped: one of them could end the paste early, and the
/// rest of the text would then be read as keys.
fn message_for_agent(review_path: &Path, comments: &str) -> String {
    let lumiere = lumiere_command();
    let path = review_path.display();
    let continues_a_thread = comments
        .lines()
        .any(|line| line.starts_with("## Thread ") && line.contains(EARLIER_COMMENTS_LABEL));
    let earlier_comments = if continues_a_thread {
        format!(" Earlier comments of a thread: {lumiere} thread show <thread-id> --repo '{path}'")
    } else {
        String::new()
    };
    let message = format!(
        "Lumiere review comments. Answer each thread below with:\n\
         {lumiere} thread reply <thread-id> --repo '{path}' --body '<reply>'\n\
         Reply in the thread first, in a few sentences, saying whether you agree and what \
         you would change. That command is your whole answer: write nothing after it. \
         Edit code only after I approve in that thread.\
         {earlier_comments}\n\n\
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
    /// The agent panes found by the last search, shown while the next one runs.
    candidates: Vec<AgentPane>,
    /// The repository or workspace folder under review, named in every message.
    review_path: PathBuf,
}

impl<'r> AgentLink<'r> {
    /// Attaches to `agent_block` in the current Rex session when given, otherwise to
    /// the most recently active agent pane in or under one of `review_roots`.
    pub fn attach(
        rex: &'r dyn Rex,
        agent_block: Option<&str>,
        current_session: Option<&str>,
        review_roots: &[PathBuf],
        review_path: PathBuf,
    ) -> Self {
        let candidates = match (agent_block, current_session) {
            (Some(block), Some(session)) => {
                let current_session = RexSession {
                    id: session.to_string(),
                    label: String::new(),
                };
                let pane = rex.terminal_pane(session, block);
                pane.and_then(|pane| agent_pane(&current_session, pane)).into_iter().collect()
            }
            (Some(_), None) => Vec::new(),
            (None, _) => find_agent_panes(rex, review_roots),
        };
        Self {
            rex,
            pane: candidates.first().cloned(),
            candidates,
            review_path,
        }
    }

    pub fn is_attached(&self) -> bool {
        self.pane.is_some()
    }

    pub fn attached_block(&self) -> Option<&str> {
        self.pane.as_ref().map(|pane| pane.block.as_str())
    }

    /// The agent panes found by the last search: nothing is asked of Rex.
    pub fn known_candidates(&self) -> &[AgentPane] {
        &self.candidates
    }

    /// Searches for agent panes again.
    pub fn find_candidates(&mut self, review_roots: &[PathBuf]) -> &[AgentPane] {
        self.candidates = find_agent_panes(self.rex, review_roots);
        &self.candidates
    }

    pub fn set_pane(&mut self, pane: Option<AgentPane>) {
        self.pane = pane;
    }

    /// Short text for the footer.
    pub fn label(&self) -> String {
        match &self.pane {
            Some(pane) if pane.status.state.is_empty() => pane.status.app.clone(),
            Some(pane) => format!("{} {}", pane.status.app, pane.status.state),
            None => "no agent".to_string(),
        }
    }

    /// Reads the agent's state again. Detaches when the pane no longer runs an agent.
    pub fn refresh(&mut self) {
        let Some(pane) = &mut self.pane else {
            return;
        };
        let now = self.rex.terminal_pane(&pane.session, &pane.block);
        match now.as_ref().and_then(running_agent) {
            Some(status) => pane.status = status.clone(),
            None => self.pane = None,
        }
    }

    /// Sends `comments` to the agent, with how to answer them. The pane is read
    /// first. When it no longer runs an agent, nothing is sent and the link is
    /// dropped: the same text pasted into a shell would be run as commands. When the
    /// agent is asking its user something, nothing is sent either.
    pub fn send_comments(&mut self, comments: &str) -> Result<(), String> {
        let Some(agent) = self.pane.as_ref().map(|pane| pane.status.app.clone()) else {
            return Err("no agent attached".to_string());
        };
        self.refresh();
        let Some(pane) = &self.pane else {
            return Err(format!("the pane no longer runs {}", agent));
        };
        if pane.status.state == BLOCKED {
            return Err(format!("{} is waiting for you", agent));
        }
        let message = message_for_agent(&self.review_path, comments);
        self.rex.submit_message(&pane.session, &pane.block, &message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const AGENT_PID: u64 = 7;
    const SHELL_PID: u64 = 1;

    /// A pane whose foreground program reported `app`, or nothing when `app` is empty.
    struct FakePane {
        session: &'static str,
        block: &'static str,
        app: &'static str,
        state: &'static str,
        cwd: &'static str,
        updated_at: &'static str,
        foreground_pid: u64,
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

    fn described(pane: &FakePane) -> RexPane {
        let status = PaneStatus {
            app: pane.app.to_string(),
            state: pane.state.to_string(),
            updated_at: pane.updated_at.to_string(),
            owner_pid: Some(AGENT_PID),
        };
        RexPane {
            block: pane.block.to_string(),
            cwd: PathBuf::from(pane.cwd),
            foreground_pid: Some(pane.foreground_pid),
            statuses: if pane.app.is_empty() { Vec::new() } else { vec![status] },
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

        fn terminal_panes(&self, session: &str) -> Vec<RexPane> {
            let panes = self.panes.borrow();
            panes.iter().filter(|pane| pane.session == session).map(described).collect()
        }

        fn terminal_pane(&self, session: &str, block: &str) -> Option<RexPane> {
            let panes = self.panes.borrow();
            let pane = panes.iter().find(|pane| pane.session == session && pane.block == block);
            pane.map(described)
        }

        fn submit_message(&self, _session: &str, block: &str, message: &str) -> Result<(), String> {
            self.typed.borrow_mut().push((block.to_string(), message.to_string()));
            Ok(())
        }
    }

    /// An idle pane in which `app` runs and reported its status.
    fn pane(
        session: &'static str,
        block: &'static str,
        app: &'static str,
        cwd: &'static str,
        updated_at: &'static str,
    ) -> FakePane {
        FakePane {
            session,
            block,
            app,
            state: "idle",
            cwd,
            updated_at,
            foreground_pid: AGENT_PID,
        }
    }

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/work/api"), PathBuf::from("/work")]
    }

    fn attach<'r>(rex: &'r FakeRex, agent_block: Option<&str>) -> AgentLink<'r> {
        AgentLink::attach(rex, agent_block, Some("s1"), &roots(), PathBuf::from("/work"))
    }

    fn blocks(panes: &[AgentPane]) -> Vec<&str> {
        panes.iter().map(|pane| pane.block.as_str()).collect()
    }

    #[test]
    fn test_attaches_to_the_latest_agent_pane_in_or_under_a_review_root() {
        let rex = FakeRex::with(vec![
            pane("s1", "older", "pi", "/work/api", "2026-11-18T10:00:00Z"),
            pane("s1", "shell", "", "/work", "2026-11-18T13:00:00Z"),
            // A program that reports a status and is not a coding agent.
            pane("s1", "build", "cargo", "/work", "2026-11-18T13:00:00Z"),
            pane("s1", "subdirectory", "codex", "/work/api/src", "2026-11-18T12:00:00Z"),
            pane("s2", "elsewhere", "pi", "/other", "2026-11-18T13:00:00Z"),
            // A sibling whose name only starts like a root.
            pane("s2", "sibling", "pi", "/workshop", "2026-11-18T13:00:00Z"),
            pane("s2", "at-the-root", "claude-code", "/work", "2026-11-18T11:00:00Z"),
        ]);

        let link = attach(&rex, None);

        assert_eq!(link.attached_block(), Some("subdirectory"));
        assert_eq!(link.label(), "codex idle");
        assert_eq!(blocks(link.known_candidates()), vec!["subdirectory", "at-the-root", "older"]);
        assert_eq!(
            link.known_candidates()[1].picker_label(),
            "claude-code · /work · label of s2 · idle"
        );
    }

    #[test]
    fn test_a_status_left_by_an_agent_that_exited_does_not_make_the_pane_an_agent() {
        let mut back_at_the_shell = pane("s1", "quit", "pi", "/work", "2026-11-18T12:00:00Z");
        back_at_the_shell.state = "done";
        back_at_the_shell.foreground_pid = SHELL_PID;
        let rex = FakeRex::with(vec![back_at_the_shell]);

        assert!(!attach(&rex, None).is_attached());
    }

    #[test]
    fn test_known_candidates_are_those_of_the_last_search() {
        let rex = FakeRex::with(vec![pane("s1", "first", "pi", "/work", "2026-11-18T10:00:00Z")]);
        let mut link = attach(&rex, None);
        rex.panes
            .borrow_mut()
            .push(pane("s1", "second", "antigravity", "/work", "2026-11-18T12:00:00Z"));

        assert_eq!(blocks(link.known_candidates()), vec!["first"]);
        assert_eq!(blocks(link.find_candidates(&roots())), vec!["second", "first"]);
        assert_eq!(blocks(link.known_candidates()), vec!["second", "first"]);
    }

    #[test]
    fn test_agent_block_is_used_without_searching_and_only_if_it_runs_an_agent() {
        let rex = FakeRex::with(vec![
            pane("s1", "given", "pi", "/somewhere/else", "2026-11-18T10:00:00Z"),
            pane("s1", "shell", "", "/work", "2026-11-18T10:00:00Z"),
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
    fn test_reading_earlier_comments_is_explained_only_when_a_thread_has_some() {
        let first_comment = "## Thread 1a2b · a.rs line 2 (RIGHT)\n\nyou: why?";
        let follow_up = "## Thread 1a2b · a.rs line 2 (RIGHT) · earlier comments: 2\n\nyou: ok";

        let without = message_for_agent(Path::new("/work"), first_comment);
        let with = message_for_agent(Path::new("/work"), follow_up);

        assert!(!without.contains("thread show"), "{}", without);
        assert!(with.contains("thread show <thread-id> --repo '/work'"), "{}", with);
    }

    #[test]
    fn test_the_agent_is_given_the_short_name_only_when_path_leads_to_this_executable() {
        let installed = tempfile::tempdir().expect("temp dir");
        let elsewhere = tempfile::tempdir().expect("temp dir");
        let executable = installed.path().join("lumiere");
        std::fs::write(&executable, "").expect("executable");
        std::fs::write(elsewhere.path().join("lumiere"), "").expect("another lumiere");
        let path_of = |directories: &[&Path]| std::env::join_paths(directories).expect("PATH");

        let on_path = path_of(&[installed.path(), elsewhere.path()]);
        let shadowed = path_of(&[elsewhere.path(), installed.path()]);

        assert_eq!(command_for(&executable, Some(&on_path)), "lumiere");
        assert_eq!(command_for(&executable, Some(&shadowed)), format!("'{}'", executable.display()));
        assert_eq!(command_for(&executable, None), format!("'{}'", executable.display()));
    }

    #[test]
    fn test_nothing_is_typed_into_a_pane_that_stopped_running_the_agent() {
        let rex = FakeRex::with(vec![pane("s1", "agent", "pi", "/work", "")]);
        let mut link = attach(&rex, None);
        // The agent quit: its last status is still there, the shell is back in front.
        rex.panes.borrow_mut()[0].foreground_pid = SHELL_PID;

        let error = link.send_comments("you: why?").expect_err("the agent quit");

        assert_eq!(error, "the pane no longer runs pi");
        assert!(rex.typed.borrow().is_empty());
        assert!(!link.is_attached());
        assert_eq!(link.label(), "no agent");
    }

    #[test]
    fn test_nothing_is_typed_while_the_agent_is_asking_its_user_something() {
        let rex = FakeRex::with(vec![pane("s1", "agent", "codex", "/work", "")]);
        let mut link = attach(&rex, None);
        // Since the last refresh: submitting now would answer the agent's question.
        rex.panes.borrow_mut()[0].state = "blocked";

        let error = link.send_comments("you: why?").expect_err("the agent is blocked");

        assert_eq!(error, "codex is waiting for you");
        assert!(rex.typed.borrow().is_empty());
        assert_eq!(link.label(), "codex blocked");
    }

    #[test]
    fn test_panes_are_read_from_what_rex_returns() {
        let panes = serde_json::json!([
            {
                "block": "block:agent",
                "process": {
                    "child": { "pid": 1, "argv0": "zsh", "cwd": "/work" },
                    "foreground": { "pid": 7, "argv0": "pi", "cwd": "/work/api" }
                },
                "status": { "records": [ {
                    "state": "working", "app": "pi", "updated_at": "2026-11-23T16:18:13Z",
                    "owner": { "pid": 7, "name": "node" }
                } ] }
            },
            {
                // Started through a wrapper as another user: no folder on the foreground.
                "block": "block:wrapped",
                "process": {
                    "child": { "pid": 1, "argv0": "zsh", "cwd": "/work" },
                    "foreground": { "pid": 9, "argv0": "bash" }
                },
                "status": { "records": {} }
            }
        ]);

        let panes = panes_from(&panes);

        assert_eq!(blocks_of(&panes), vec!["block:agent", "block:wrapped"]);
        assert_eq!(panes[0].cwd, PathBuf::from("/work/api"));
        let agent = running_agent(&panes[0]).expect("an agent runs in the first pane");
        assert_eq!((agent.app.as_str(), agent.state.as_str()), ("pi", "working"));
        assert_eq!(panes[1].cwd, PathBuf::from("/work"));
        assert!(running_agent(&panes[1]).is_none());
        assert!(panes_from(&serde_json::json!({})).is_empty());
    }

    fn blocks_of(panes: &[RexPane]) -> Vec<&str> {
        panes.iter().map(|pane| pane.block.as_str()).collect()
    }
}
