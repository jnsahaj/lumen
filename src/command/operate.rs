use spinoff::{spinners, Color, Spinner};
use std::io::{self, Write};
use thiserror::Error;

#[derive(Debug)]
pub struct OperateResult {
    pub command: String,
    pub explanation: String,
    pub warning: Option<String>,
}

#[derive(Error, Debug)]
#[error("Failed to extract {field} from AI response: {message}")]
pub struct ExtractError {
    field: String,
    message: String,
}

use crate::{error::LumenError, provider::LumenProvider};

use super::LumenCommand;

pub struct OperateCommand {
    pub query: String,
}

pub fn extract_operate_response(ai_response: &str) -> Result<OperateResult, ExtractError> {
    // Helper function to extract content between XML tags
    fn extract_tag(text: &str, tag_name: &str) -> Result<String, ExtractError> {
        let start_tag = format!("<{}>", tag_name);
        let end_tag = format!("</{}>", tag_name);

        if let Some(start) = text.find(&start_tag) {
            if let Some(end) = text.find(&end_tag) {
                let content_start = start + start_tag.len();
                if content_start < end {
                    return Ok(text[content_start..end].trim().to_string());
                }
            }
        }

        Err(ExtractError {
            field: tag_name.to_string(),
            message: format!("Could not find valid <{}> tags", tag_name),
        })
    }

    // Extract required fields
    let command = extract_tag(ai_response, "command")?;
    let explanation = extract_tag(ai_response, "explanation")?;

    // Warning is optional
    let warning = match extract_tag(ai_response, "warning") {
        Ok(warning_text) if !warning_text.is_empty() => Some(warning_text),
        _ => None,
    };

    Ok(OperateResult {
        command,
        explanation,
        warning,
    })
}

pub fn process_operation(result: OperateResult) -> Result<(), io::Error> {
    // Display the explanation
    println!("\n--- What this will do ---");
    println!("{}", result.explanation);

    // Display warnings if any and prompt for confirmation
    if let Some(warning) = result.warning {
        // print warning in yellow colour
        println!("\n\x1b[33mWarning: {}\x1b[0m", warning);
    }

    print!("\n{} [y/N] ", result.command);
    io::stdout().flush()?; // Ensure prompt is shown immediately

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    println!();

    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Operation canceled.");
        return Ok(());
    }

    // Using a shell to execute the git command
    #[cfg(target_family = "unix")]
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&result.command)
        .output()?;

    #[cfg(target_family = "windows")]
    let output = std::process::Command::new("cmd")
        .arg("/C")
        .arg(&result.command)
        .output()?;

    // Print command output
    if !output.stdout.is_empty() {
        io::stdout().write_all(&output.stdout)?;
    }

    if !output.stderr.is_empty() {
        io::stderr().write_all(&output.stderr)?;
    }

    if !output.status.success() {
        eprintln!(
            "\nCommand failed with exit code: {:?}",
            output.status.code()
        );
    }

    Ok(())
}

impl OperateCommand {
    pub async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        LumenCommand::print_with_mdcat(format!("`query`: {}", &self.query))?;

        let spinner_text = "Generating answer...".to_string();

        let mut spinner = Spinner::new(spinners::Dots, spinner_text, Color::Blue);
        let result = provider.operate(self).await?;
        let operate_result = extract_operate_response(&result)
            .map_err(|e| LumenError::CommandError(e.to_string()))?;
        spinner.success("Done");

        process_operation(operate_result)?;
        Ok(())
    }
}
