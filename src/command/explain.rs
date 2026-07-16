use spinoff::{spinners, Color, Spinner};

use crate::{
    error::LumenError,
    git_entity::{diff::Diff, GitEntity},
    provider::LumenProvider,
};

use super::LumenCommand;

pub struct ExplainCommand {
    pub git_entity: GitEntity,
    pub query: Option<String>,
    pub grouped: bool,
}

/// Extract the raw diff text out of a [`GitEntity`], regardless of variant.
fn diff_text(git_entity: &GitEntity) -> &str {
    match git_entity {
        GitEntity::Commit(commit) => &commit.diff,
        GitEntity::Diff(Diff::WorkingTree { diff, .. } | Diff::CommitsRange { diff, .. }) => diff,
    }
}

impl ExplainCommand {
    fn diff_text(&self) -> &str {
        diff_text(&self.git_entity)
    }

    pub async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        LumenCommand::print_with_mdcat(self.git_entity.format_static_details(provider))?;
        if let Some(query) = &self.query {
            LumenCommand::print_with_mdcat(format!("`query`: {query}"))?;
        }

        let spinner_text = if self.grouped {
            "Grouping changes...".to_string()
        } else {
            match &self.query {
                Some(_) => "Generating answer...".to_string(),
                None => "Generating summary...".to_string(),
            }
        };

        let mut spinner = Spinner::new(spinners::Dots, spinner_text, Color::Blue);
        let result = if self.grouped {
            provider.explain_grouped(self).await?
        } else {
            provider.explain(self).await?
        };
        spinner.success("Done");

        if self.grouped {
            let mut summary = crate::grouped_summary::parse_grouped_summary(&result)?;
            let ground_truth = crate::grouped_summary::files_from_unified_diff(self.diff_text());
            let report = crate::grouped_summary::reconcile_groups(&mut summary, &ground_truth);
            if !report.is_clean() {
                eprintln!(
                    "note: grouping adjusted ({} unknown, {} ungrouped file(s))",
                    report.unknown_files.len(),
                    report.ungrouped_files.len()
                );
            }
            LumenCommand::print_with_mdcat(crate::grouped_summary::render_markdown(&summary))?;
        } else {
            LumenCommand::print_with_mdcat(result)?;
        }
        Ok(())
    }
}
