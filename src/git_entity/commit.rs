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
    /// Build a commit object from VCS backend CommitInfo.
    pub fn from_commit_info(info: crate::vcs::CommitInfo) -> Self {
        // Parse author into name and email
        let (author_name, author_email) = if let Some(idx) = info.author.find(" <") {
            let name = info.author[..idx].to_string();
            let email = info.author[idx + 2..].trim_end_matches('>').to_string();
            (name, email)
        } else {
            (info.author.clone(), String::new())
        };

        Commit {
            full_hash: info.commit_id,
            message: info.message,
            diff: info.diff,
            author_name,
            author_email,
            date: info.date,
        }
    }
}
