mod app;
mod context;
mod diff_algo;
mod git;
pub mod highlight;
mod render;
mod search;
mod state;
mod sticky_lines;
pub mod theme;
mod types;
mod watcher;

use std::io;

use crate::commit_reference::CommitReference;



pub struct DiffOptions {
    pub reference: Option<CommitReference>,
    pub file: Option<Vec<String>>,
    pub watch: bool,
}

pub fn run_diff_ui(options: DiffOptions) -> io::Result<()> {
    app::run_app(options)
}
