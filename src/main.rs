use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::io::{self, Write};
use std::process::Command;

const MAX_DIFF_CHARS: usize = 3072;

// ---------- Domain types ----------
#[derive(Debug, Serialize, Deserialize)]
struct Commit {
    #[serde(default)]
    r#type: String, // feat, fix, docs, etc.
    #[serde(default)]
    scope: String, // optional component
    #[serde(default)]
    message: String, // 50 chars max per prompt
}

// ---------- Git ----------
fn stage_all_changes() -> Result<()> {
    let status = Command::new("git")
        .args(["add", "."])
        .status()
        .context("failed to run `git add .`")?;

    if !status.success() {
        return Err(anyhow!("git add failed with status: {}", status));
    }

    Ok(())
}

fn get_staged_changes() -> Result<String> {
    let output = Command::new("git")
        .args(["diff", "--cached", "-b"])
        .output()
        .context("failed to run `git diff --cached -b`")?;

    if !output.status.success() {
        return Err(anyhow!("git diff failed with status: {}", output.status));
    }

    let mut diff = String::from_utf8(output.stdout).context("git output was not valid UTF-8")?;

    if diff.trim().is_empty() {
        return Err(anyhow!("no staged changes found"));
    }

    if diff.chars().count() > MAX_DIFF_CHARS {
        let truncated: String = diff.chars().take(MAX_DIFF_CHARS).collect();
        diff = format!("{truncated}\n... (truncated)");
    }

    Ok(diff)
}

// ---------- OpenAI Chat Completions request/response ----------
#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    r#type: String,
    // json_schema is supported for structured outputs. If your account/region
    // lacks this feature, you can omit `response_format` entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    json_schema: Option<JsonSchema>,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    schema: serde_json::Value,
    // force the model to only output the object (no extra text)
    strict: bool,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    content: String,
}

// ---------- LLM ----------
async fn generate_message(changes: &str) -> Result<Commit> {
    let api_key = env::var("OPENAI_API_KEY").context("OPENAI_API_KEY env var is not set")?;
    let base =
        env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model = env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4.1-mini".to_string());

    // System + user messages; user holds the diff.
    let system = r#"You are a git commit message generator.
Analyze changes and output JSON with:
- type: feat|fix|docs|style|refactor|test|chore
- scope: affected component (optional)
- message: clear description (50 chars max)
Return ONLY valid JSON, no other text."#;

    let user = format!("Changes:\n{changes}");

    // JSON Schema to enforce structure (Structured Outputs).
    let schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["type", "scope", "message"],  // <- include "scope"
        "properties": {
            "type":   { "type": "string", "enum": ["feat","fix","docs","style","refactor","test","chore"] },
            "scope":  { "type": "string" },  // model can output "" if nothing fits
            "message":{ "type": "string", "maxLength": 50 }
        }
    });

    let req = ChatRequest {
        model,
        messages: vec![
            Message {
                role: "system".into(),
                content: system.into(),
            },
            Message {
                role: "user".into(),
                content: user,
            },
        ],
        temperature: 0.0,
        response_format: Some(ResponseFormat {
            r#type: "json_schema".into(), // fallback: use "json_object" if json_schema isn't enabled
            json_schema: Some(JsonSchema {
                name: "commit_message".into(),
                schema,
                strict: true,
            }),
        }),
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/chat/completions"))
        .bearer_auth(api_key)
        .json(&req)
        .send()
        .await
        .context("OpenAI request failed")?;

    // Check status first; only consume the body on the error path.
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default(); // consumes resp
        return Err(anyhow!(
            "OpenAI request failed with status {}: {}",
            status,
            text
        ));
    }

    // If we got here, resp is still available and unconsumed.
    let parsed: ChatResponse = resp
        .json()
        .await
        .context("failed to parse OpenAI response")?;

    let content = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no choices returned"))?
        .message
        .content;

    // Model should have returned strict JSON per schema.
    let commit: Commit = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse commit JSON (raw: {content:?})"))?;

    Ok(commit)
}

fn build_commit_line(commit: &Commit) -> String {
    let mut out = commit.r#type.trim().to_string();
    if !commit.scope.trim().is_empty() {
        out.push('(');
        out.push_str(commit.scope.trim());
        out.push(')');
    }
    out.push_str(": ");
    out.push_str(commit.message.trim());
    out
}

fn confirm_push() -> Result<bool> {
    loop {
        eprint!("Push commit to remote? (y/n): ");
        io::stderr().flush()?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("failed to read user input")?;

        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {
                eprintln!("Please answer 'y' or 'n'");
                continue;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    match stage_all_changes() {
        Ok(_) => eprintln!("Staged all changes with `git add .`"),
        Err(e) => {
            eprintln!("Failed to stage changes: {e}");
            std::process::exit(1);
        }
    };

    let changes = match get_staged_changes() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to get staged changes: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("Staged diff found; generating message for changes...");

    let commit = match generate_message(&changes).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to generate commit message: {e}");
            std::process::exit(1);
        }
    };

    eprintln!(
        "Parsed commit: type='{}', scope='{}', message='{}'",
        commit.r#type, commit.scope, commit.message
    );

    let line = build_commit_line(&commit);

    // Run: git commit -e -m "<line>"
    let status = Command::new("git")
        .args(["commit", "-e", "-m", &line])
        .status()
        .context("failed to run `git commit`")?;

    if !status.success() {
        return Err(anyhow!("git commit failed with status: {status}"));
    }

    eprintln!("Commit created successfully.");

    // Ask for confirmation before pushing
    let should_push = match confirm_push() {
        Ok(confirmed) => confirmed,
        Err(e) => {
            eprintln!("Error during push confirmation: {e}");
            std::process::exit(1);
        }
    };

    if !should_push {
        eprintln!("Push cancelled. Commit saved locally.");
        return Ok(());
    }

    // Run: git push
    let status = Command::new("git")
        .args(["push"])
        .status()
        .context("failed to run `git push`")?;

    if !status.success() {
        return Err(anyhow!("git push failed with status: {status}"));
    }

    eprintln!("Changes pushed successfully!");

    Ok(())
}
