//! Coding agent integration through Rex: pane discovery via OSC 7501 status records,
//! non-blocking background polling, and safe bracketed-paste delivery.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use serde_json::Value;

use super::thread::EARLIER_COMMENTS_LABEL;

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

const BLOCKED: &str = "blocked";
const PROGRAM_NAME: &str = "lumiere";

pub struct RexSession {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct PaneStatus {
    pub app: String,
    pub state: String,
    pub updated_at: String,
    pub owner_pid: Option<u64>,
}

pub struct RexPane {
    pub block: String,
    pub cwd: PathBuf,
    pub foreground_pid: Option<u64>,
    pub statuses: Vec<PaneStatus>,
}

pub trait Rex: Send + Sync {
    fn sessions(&self) -> Vec<RexSession>;
    fn terminal_panes(&self, session: &str) -> Vec<RexPane>;
    fn terminal_pane(&self, session: &str, block: &str) -> Option<RexPane>;
    fn submit_message(&self, session: &str, block: &str, message: &str) -> Result<(), String>;
}

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

#[derive(Clone, Copy)]
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
        Self::json(&args)
            .map(|panes| panes_from(&panes))
            .unwrap_or_default()
    }

    fn run(args: &[&str], input: &str) -> Result<(), String> {
        let cannot_run = |error: std::io::Error| format!("cannot run rex: {error}");
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
    panes
        .as_array()
        .into_iter()
        .flatten()
        .map(pane_from)
        .collect()
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
        // Bracketed paste delivers newlines as prompt text rather than submitting early.
        let paste = format!("\x1b[200~{message}\x1b[201~");
        Self::run(&["send", "-s", session, "-b", block], &paste)?;
        Self::run(&["send-key", "-s", session, "-b", block, "enter"], "")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPane {
    pub session: String,
    pub session_label: String,
    pub block: String,
    pub cwd: PathBuf,
    pub status: PaneStatus,
}

impl AgentPane {
    pub fn picker_label(&self) -> String {
        let cwd = self.cwd.display().to_string();
        [
            &self.status.app,
            &self.session_label,
            &self.status.state,
            &cwd,
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

/// Only statuses reported by the current foreground PID count so exited agents never
/// let comments get pasted into an interactive shell.
fn running_agent(pane: &RexPane) -> Option<&PaneStatus> {
    pane.statuses.iter().find(|status| {
        KNOWN_AGENTS.contains(&status.app.as_str())
            && status.owner_pid.is_some()
            && status.owner_pid == pane.foreground_pid
    })
}

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

fn discover_initial_candidates(
    rex: &dyn Rex,
    agent_block: Option<&str>,
    current_session: Option<&str>,
    review_roots: &[PathBuf],
) -> Vec<AgentPane> {
    match (agent_block, current_session) {
        (Some(block), Some(session)) => {
            let current_session = RexSession {
                id: session.to_string(),
                label: String::new(),
            };
            rex.terminal_pane(session, block)
                .and_then(|pane| agent_pane(&current_session, pane))
                .into_iter()
                .collect()
        }
        (Some(_), None) => Vec::new(),
        (None, _) => find_agent_panes(rex, review_roots),
    }
}

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
         Reply in the thread first, saying whether you agree and what \
         you would change. That command is your whole answer: write nothing after it. \
         Edit code only after I approve in that thread.\
         {earlier_comments}\n\n\
         {comments}"
    );
    // Strip control characters so none can terminate the bracketed paste sequence early.
    message
        .chars()
        .filter(|character| *character == '\n' || !character.is_control())
        .collect()
}

pub struct AgentLink<'r> {
    rex: &'r dyn Rex,
    pane: Option<AgentPane>,
    candidates: Vec<AgentPane>,
    review_path: PathBuf,
    searched: bool,
    user_selected: bool,
}

impl<'r> AgentLink<'r> {
    pub fn new(rex: &'r dyn Rex, review_path: PathBuf) -> Self {
        Self {
            rex,
            pane: None,
            candidates: Vec::new(),
            review_path,
            searched: false,
            user_selected: false,
        }
    }

    pub fn has_searched(&self) -> bool {
        self.searched
    }

    #[cfg(test)]
    pub fn attach(
        &mut self,
        agent_block: Option<&str>,
        current_session: Option<&str>,
        review_roots: &[PathBuf],
    ) {
        let candidates =
            discover_initial_candidates(self.rex, agent_block, current_session, review_roots);
        self.apply_attached(candidates);
    }

    pub fn apply_attached(&mut self, candidates: Vec<AgentPane>) {
        if !self.user_selected {
            self.pane = candidates.first().cloned();
        }
        self.candidates = candidates;
        self.searched = true;
    }

    pub fn is_attached(&self) -> bool {
        self.pane.is_some()
    }

    pub fn attached_block(&self) -> Option<&str> {
        self.pane.as_ref().map(|pane| pane.block.as_str())
    }

    pub fn attached_target(&self) -> Option<(String, String)> {
        self.pane
            .as_ref()
            .map(|pane| (pane.session.clone(), pane.block.clone()))
    }

    pub fn known_candidates(&self) -> &[AgentPane] {
        &self.candidates
    }

    pub fn apply_candidates(&mut self, candidates: Vec<AgentPane>) {
        self.candidates = candidates;
        self.searched = true;
    }

    pub fn set_pane(&mut self, pane: Option<AgentPane>) {
        self.pane = pane;
        self.user_selected = true;
    }

    pub fn label(&self) -> String {
        match &self.pane {
            Some(pane) if pane.status.state.is_empty() => pane.status.app.clone(),
            Some(pane) => format!("{} {}", pane.status.app, pane.status.state),
            None if !self.searched => "finding agent".to_string(),
            None => "no agent".to_string(),
        }
    }

    pub fn refresh(&mut self) {
        let Some(pane) = &mut self.pane else {
            return;
        };
        let now = self.rex.terminal_pane(&pane.session, &pane.block);
        let status = now.as_ref().and_then(running_agent).cloned();
        let (session, block) = (pane.session.clone(), pane.block.clone());
        self.apply_refresh(&session, &block, status);
    }

    /// Applies an asynchronous refresh only if the same pane is still attached.
    /// Returns true if the visible state changed.
    pub fn apply_refresh(
        &mut self,
        session: &str,
        block: &str,
        status: Option<PaneStatus>,
    ) -> bool {
        let Some(pane) = &mut self.pane else {
            return false;
        };
        if pane.session != session || pane.block != block {
            return false;
        }
        match status {
            Some(next) if next != pane.status => {
                pane.status = next;
                true
            }
            Some(_) => false,
            None => {
                self.pane = None;
                true
            }
        }
    }

    /// Reads the pane live before pasting so we never type into a shell or blocked prompt.
    pub fn send_comments(&mut self, comments: &str) -> Result<(), String> {
        let Some(agent) = self.pane.as_ref().map(|pane| pane.status.app.clone()) else {
            return Err("no agent attached".to_string());
        };
        self.refresh();
        let Some(pane) = &self.pane else {
            return Err(format!("the pane no longer runs {agent}"));
        };
        if pane.status.state == BLOCKED {
            return Err(format!("{agent} is waiting for you"));
        }
        let message = message_for_agent(&self.review_path, comments);
        self.rex
            .submit_message(&pane.session, &pane.block, &message)
    }
}

pub enum AgentRequest {
    Attach {
        agent_block: Option<String>,
        current_session: Option<String>,
        review_roots: Vec<PathBuf>,
    },
    Refresh {
        session: String,
        block: String,
    },
    FindCandidates {
        review_roots: Vec<PathBuf>,
    },
}

pub enum AgentResponse {
    Attached(Vec<AgentPane>),
    Refreshed {
        session: String,
        block: String,
        status: Option<PaneStatus>,
    },
    Candidates(Vec<AgentPane>),
}

/// Runs Rex discovery and status polling off the UI thread so startup and periodic
/// refreshes never stall keyboard input.
pub struct AgentWorker {
    tx: Sender<AgentRequest>,
    rx: Receiver<AgentResponse>,
}

impl AgentWorker {
    pub fn spawn(rex: Box<dyn Rex>) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<AgentRequest>();
        let (res_tx, res_rx) = mpsc::channel::<AgentResponse>();

        thread::spawn(move || {
            while let Ok(request) = req_rx.recv() {
                let response = match request {
                    AgentRequest::Attach {
                        agent_block,
                        current_session,
                        review_roots,
                    } => AgentResponse::Attached(discover_initial_candidates(
                        rex.as_ref(),
                        agent_block.as_deref(),
                        current_session.as_deref(),
                        &review_roots,
                    )),
                    AgentRequest::Refresh { session, block } => {
                        let status = rex
                            .terminal_pane(&session, &block)
                            .as_ref()
                            .and_then(running_agent)
                            .cloned();
                        AgentResponse::Refreshed {
                            session,
                            block,
                            status,
                        }
                    }
                    AgentRequest::FindCandidates { review_roots } => {
                        AgentResponse::Candidates(find_agent_panes(rex.as_ref(), &review_roots))
                    }
                };
                if res_tx.send(response).is_err() {
                    break;
                }
            }
        });

        Self {
            tx: req_tx,
            rx: res_rx,
        }
    }

    pub fn request(&self, req: AgentRequest) {
        let _ = self.tx.send(req);
    }

    pub fn try_recv(&self) -> Option<AgentResponse> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;

    const AGENT_PID: u64 = 7;
    const SHELL_PID: u64 = 1;

    #[derive(Clone)]
    struct FakePane {
        session: &'static str,
        block: &'static str,
        app: &'static str,
        state: &'static str,
        cwd: &'static str,
        updated_at: &'static str,
        foreground_pid: u64,
    }

    #[derive(Clone)]
    struct FakeRex {
        panes: Arc<Mutex<Vec<FakePane>>>,
        typed: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl FakeRex {
        fn with(panes: Vec<FakePane>) -> Self {
            Self {
                panes: Arc::new(Mutex::new(panes)),
                typed: Arc::new(Mutex::new(Vec::new())),
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
            statuses: if pane.app.is_empty() {
                Vec::new()
            } else {
                vec![status]
            },
        }
    }

    impl Rex for FakeRex {
        fn sessions(&self) -> Vec<RexSession> {
            let panes = self.panes.lock().unwrap();
            let mut ids: Vec<&str> = panes.iter().map(|pane| pane.session).collect();
            ids.dedup();
            ids.into_iter()
                .map(|id| RexSession {
                    id: id.to_string(),
                    label: format!("label of {id}"),
                })
                .collect()
        }

        fn terminal_panes(&self, session: &str) -> Vec<RexPane> {
            self.panes
                .lock()
                .unwrap()
                .iter()
                .filter(|pane| pane.session == session)
                .map(described)
                .collect()
        }

        fn terminal_pane(&self, session: &str, block: &str) -> Option<RexPane> {
            self.panes
                .lock()
                .unwrap()
                .iter()
                .find(|pane| pane.session == session && pane.block == block)
                .map(described)
        }

        fn submit_message(&self, _session: &str, block: &str, message: &str) -> Result<(), String> {
            self.typed
                .lock()
                .unwrap()
                .push((block.to_string(), message.to_string()));
            Ok(())
        }
    }

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
        let mut link = AgentLink::new(rex, PathBuf::from("/work"));
        link.attach(agent_block, Some("s1"), &roots());
        link
    }

    fn blocks(panes: &[AgentPane]) -> Vec<&str> {
        panes.iter().map(|pane| pane.block.as_str()).collect()
    }

    #[test]
    fn test_attaches_to_the_latest_agent_pane_in_or_under_a_review_root() {
        let rex = FakeRex::with(vec![
            pane("s1", "older", "pi", "/work/api", "2026-11-18T10:00:00Z"),
            pane("s1", "shell", "", "/work", "2026-11-18T13:00:00Z"),
            pane("s1", "build", "cargo", "/work", "2026-11-18T13:00:00Z"),
            pane(
                "s1",
                "subdirectory",
                "codex",
                "/work/api/src",
                "2026-11-18T12:00:00Z",
            ),
            pane("s2", "elsewhere", "pi", "/other", "2026-11-18T13:00:00Z"),
            pane("s2", "sibling", "pi", "/workshop", "2026-11-18T13:00:00Z"),
            pane(
                "s2",
                "at-the-root",
                "claude-code",
                "/work",
                "2026-11-18T11:00:00Z",
            ),
        ]);

        let link = attach(&rex, None);

        assert_eq!(link.attached_block(), Some("subdirectory"));
        assert_eq!(link.label(), "codex idle");
        assert_eq!(
            blocks(link.known_candidates()),
            vec!["subdirectory", "at-the-root", "older"]
        );
        assert_eq!(
            link.known_candidates()[1].picker_label(),
            "claude-code · label of s2 · idle · /work"
        );
    }

    #[test]
    fn test_exited_agent_and_agent_block_guards() {
        let mut back_at_shell = pane("s1", "quit", "pi", "/work", "2026-11-18T12:00:00Z");
        back_at_shell.state = "done";
        back_at_shell.foreground_pid = SHELL_PID;
        let rex = FakeRex::with(vec![back_at_shell]);
        assert!(!attach(&rex, None).is_attached());

        let rex = FakeRex::with(vec![
            pane(
                "s1",
                "given",
                "pi",
                "/somewhere/else",
                "2026-11-18T10:00:00Z",
            ),
            pane("s1", "shell", "", "/work", "2026-11-18T10:00:00Z"),
            pane("s1", "matching", "pi", "/work", "2026-11-18T12:00:00Z"),
        ]);
        assert_eq!(attach(&rex, Some("given")).attached_block(), Some("given"));
        assert_eq!(attach(&rex, Some("shell")).attached_block(), None);
    }

    #[test]
    fn test_label_lifecycle_and_send_safety() {
        let rex = FakeRex::with(vec![pane("s1", "agent", "codex", "/work", "")]);
        let mut link = AgentLink::new(&rex, PathBuf::from("/work"));
        assert!(!link.has_searched());
        assert_eq!(link.label(), "finding agent");

        link.attach(None, Some("s1"), &roots());
        assert_eq!(link.label(), "codex idle");

        // Blocked agent refuses send without typing.
        rex.panes.lock().unwrap()[0].state = "blocked";
        assert_eq!(
            link.send_comments("you: why?").unwrap_err(),
            "codex is waiting for you"
        );
        assert!(rex.typed.lock().unwrap().is_empty());
        assert_eq!(link.label(), "codex blocked");

        // Unblocked agent receives sanitized message.
        rex.panes.lock().unwrap()[0].state = "idle";
        let comments = "## Thread 1a2b · a.rs line 2 (RIGHT)\n\nyou: why?\x1b[201~";
        assert_eq!(link.send_comments(comments), Ok(()));
        let typed = rex.typed.lock().unwrap();
        assert_eq!(typed.len(), 1);
        assert!(typed[0]
            .1
            .ends_with("## Thread 1a2b · a.rs line 2 (RIGHT)\n\nyou: why?[201~"));
        drop(typed);

        // Exited agent drops link without typing.
        rex.panes.lock().unwrap()[0].foreground_pid = SHELL_PID;
        assert_eq!(
            link.send_comments("you: more?").unwrap_err(),
            "the pane no longer runs codex"
        );
        assert!(!link.is_attached());
        assert_eq!(link.label(), "no agent");
    }

    #[test]
    fn test_message_earlier_comments_and_command_resolution() {
        let first = "## Thread 1a2b · a.rs line 2 (RIGHT)\n\nyou: why?";
        let follow_up = "## Thread 1a2b · a.rs line 2 (RIGHT) · earlier comments: 2\n\nyou: ok";
        assert!(!message_for_agent(Path::new("/work"), first).contains("thread show"));
        assert!(message_for_agent(Path::new("/work"), follow_up)
            .contains("thread show <thread-id> --repo '/work'"));

        let installed = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let executable = installed.path().join("lumiere");
        std::fs::write(&executable, "").unwrap();
        std::fs::write(elsewhere.path().join("lumiere"), "").unwrap();
        let on_path = std::env::join_paths([installed.path(), elsewhere.path()]).unwrap();
        let shadowed = std::env::join_paths([elsewhere.path(), installed.path()]).unwrap();

        assert_eq!(command_for(&executable, Some(&on_path)), "lumiere");
        assert_eq!(
            command_for(&executable, Some(&shadowed)),
            format!("'{}'", executable.display())
        );
        assert_eq!(
            command_for(&executable, None),
            format!("'{}'", executable.display())
        );
    }

    #[test]
    fn test_agent_worker_handles_attach_refresh_and_ignores_stale_pane_refresh() {
        let rex = FakeRex::with(vec![
            pane("s1", "first", "pi", "/work", "2026-11-18T10:00:00Z"),
            pane("s1", "second", "codex", "/work", "2026-11-18T12:00:00Z"),
        ]);
        let worker = AgentWorker::spawn(Box::new(rex.clone()));
        let mut link = AgentLink::new(&rex, PathBuf::from("/work"));

        worker.request(AgentRequest::Attach {
            agent_block: None,
            current_session: Some("s1".to_string()),
            review_roots: roots(),
        });

        let AgentResponse::Attached(candidates) =
            worker.rx.recv_timeout(Duration::from_secs(2)).unwrap()
        else {
            panic!("expected Attached");
        };
        link.apply_attached(candidates);
        assert_eq!(link.attached_block(), Some("second"));

        // Refresh runs asynchronously and detects state transition.
        rex.panes.lock().unwrap()[1].state = "working";
        worker.request(AgentRequest::Refresh {
            session: "s1".to_string(),
            block: "second".to_string(),
        });
        let AgentResponse::Refreshed {
            session,
            block,
            status,
        } = worker.rx.recv_timeout(Duration::from_secs(2)).unwrap()
        else {
            panic!("expected Refreshed");
        };
        assert!(link.apply_refresh(&session, &block, status));
        assert_eq!(link.label(), "codex working");

        // A stale refresh for a different pane does not overwrite the attached pane.
        assert!(!link.apply_refresh("s1", "first", None));
        assert_eq!(link.attached_block(), Some("second"));
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
                "block": "block:wrapped",
                "process": {
                    "child": { "pid": 1, "argv0": "zsh", "cwd": "/work" },
                    "foreground": { "pid": 9, "argv0": "bash" }
                },
                "status": { "records": {} }
            }
        ]);

        let panes = panes_from(&panes);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].cwd, PathBuf::from("/work/api"));
        let agent = running_agent(&panes[0]).unwrap();
        assert_eq!(
            (agent.app.as_str(), agent.state.as_str()),
            ("pi", "working")
        );
        assert_eq!(panes[1].cwd, PathBuf::from("/work"));
        assert!(running_agent(&panes[1]).is_none());
        assert!(panes_from(&serde_json::json!({})).is_empty());
    }
}
