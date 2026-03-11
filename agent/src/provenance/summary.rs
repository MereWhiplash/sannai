use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;
use serde::Serialize;

use super::attribution::DiffAttribution;
use super::interaction::Interaction;
use super::lineage::FileLineage;

#[derive(Debug, Clone)]
pub struct SummaryConfig {
    pub enabled: bool,
    pub command: String,
    pub max_length: usize,
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self { enabled: false, command: String::new(), max_length: 500 }
    }
}

#[derive(Debug, Serialize)]
pub struct ProvenanceBundle {
    pub interactions: Vec<Interaction>,
    pub lineage: Vec<FileLineage>,
    pub attributions: Vec<DiffAttribution>,
    pub diff: String,
}

const PROMPT_TEMPLATE: &str = r#"You are reviewing the AI-assisted development process behind a pull request. Below is the provenance data: the sequence of AI interactions, which files were read and written, and how the diff maps to specific AI interactions.

Summarize this for a code reviewer. Focus on:
- What the developer was trying to accomplish
- How many iterations it took and whether there were struggles
- Which parts of the diff were AI-generated vs manually written
- Any concerns a reviewer should pay attention to

Be concise. Write 3-5 sentences max.

<provenance>
{provenance_json}
</provenance>"#;

pub fn generate_summary(bundle: &ProvenanceBundle, config: &SummaryConfig) -> Option<String> {
    if !config.enabled || config.command.is_empty() {
        return None;
    }

    let json = match serde_json::to_string_pretty(bundle) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("Failed to serialize provenance bundle: {}", e);
            return None;
        }
    };

    let prompt = PROMPT_TEMPLATE.replace("{provenance_json}", &json);

    match run_llm_command(&config.command, &prompt) {
        Ok(output) => Some(output),
        Err(e) => {
            tracing::warn!("LLM summary generation failed: {}", e);
            None
        }
    }
}

fn run_llm_command(command: &str, prompt: &str) -> Result<String> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        anyhow::bail!("Empty LLM command");
    }

    let mut child = Command::new(parts[0])
        .args(&parts[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("LLM command failed: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
