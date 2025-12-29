use async_trait::async_trait;
use spinoff::{spinners, Color, Spinner};
use std::io::{self, Write};
use thiserror::Error;
use xml::reader::{EventReader, XmlEvent};

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

use super::{Command, LumenCommand};

pub struct OperateCommand {
    pub query: String,
}

pub fn extract_operate_response(ai_response: &str) -> Result<OperateResult, ExtractError> {
    let parser = EventReader::from_str(ai_response);
    let mut command = None;
    let mut explanation = None;
    let mut warning = None;
    let mut current_element = None;
    let mut current_text = String::new();

    for event in parser {
        match event {
            Ok(XmlEvent::StartElement { name, .. }) => {
                current_element = Some(name.local_name.clone());
                current_text.clear();
            }
            Ok(XmlEvent::Characters(text)) => {
                if current_element.is_some() {
                    current_text.push_str(&text);
                }
            }
            Ok(XmlEvent::EndElement { name }) => {
                if let Some(element) = &current_element {
                    if element == &name.local_name {
                        match element.as_str() {
                            "command" => command = Some(current_text.trim().to_string()),
                            "explanation" => explanation = Some(current_text.trim().to_string()),
                            "warning" => {
                                let trimmed = current_text.trim();
                                if !trimmed.is_empty() {
                                    warning = Some(trimmed.to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                current_element = None;
                current_text.clear();
            }
            Err(e) => {
                return Err(ExtractError {
                    field: "xml".to_string(),
                    message: format!("XML parsing error: {}", e),
                });
            }
            _ => {}
        }
    }

    let command = command.ok_or_else(|| ExtractError {
        field: "command".to_string(),
        message: "Missing <command> tag".to_string(),
    })?;

    let explanation = explanation.ok_or_else(|| ExtractError {
        field: "explanation".to_string(),
        message: "Missing <explanation> tag".to_string(),
    })?;

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

#[async_trait]
impl Command for OperateCommand {
    async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
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
