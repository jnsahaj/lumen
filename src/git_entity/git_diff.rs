#[derive(Clone, Debug)]
pub struct GitDiff {
    pub diff: String,
}

impl GitDiff {
    pub fn new(diff: String) -> Self {
        GitDiff { diff }
    }

    pub fn get_staged_diff() -> String {
        let output = std::process::Command::new("git")
            .args(["diff", "--staged"])
            .output()
            .expect("failed to execute process");

        String::from_utf8(output.stdout).unwrap()
    }
}
