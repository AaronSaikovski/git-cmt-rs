use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::env;
use std::io::{self, Write};
use std::process::{Command, Stdio};

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

fn current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .context("failed to run `git symbolic-ref`")?;
    if !output.status.success() {
        return Err(anyhow!(
            "could not determine current branch (detached HEAD?)"
        ));
    }
    Ok(String::from_utf8(output.stdout)
        .context("git output was not valid UTF-8")?
        .trim()
        .to_string())
}

fn has_upstream() -> bool {
    Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponseFormat {
    JsonObject,
    JsonSchema { json_schema: JsonSchema },
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    schema: serde_json::Value,
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

// Default is `json_object` so the tool works against Ollama and most local
// proxies; hosted OpenAI users can opt back into strict schema validation
// with `json_schema`.
fn build_response_format(
    raw: Option<&str>,
    schema: serde_json::Value,
) -> Result<Option<ResponseFormat>> {
    match raw.unwrap_or("json_object").trim().to_lowercase().as_str() {
        "json_object" => Ok(Some(ResponseFormat::JsonObject)),
        "json_schema" => Ok(Some(ResponseFormat::JsonSchema {
            json_schema: JsonSchema {
                name: "commit_message".into(),
                schema,
                strict: true,
            },
        })),
        "none" => Ok(None),
        other => Err(anyhow!(
            "OPENAI_RESPONSE_FORMAT must be one of: json_object, json_schema, none (got: {other:?})"
        )),
    }
}

// ---------- LLM ----------
async fn generate_message(changes: &str) -> Result<Commit> {
    // API key is optional: local backends like Ollama ignore auth, and some
    // proxies reject an empty `Authorization: Bearer` header.
    let api_key = env::var("OPENAI_API_KEY").ok().filter(|k| !k.is_empty());
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

    let schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["type", "scope", "message"],
        "properties": {
            "type":   { "type": "string", "enum": ["feat","fix","docs","style","refactor","test","chore"] },
            "scope":  { "type": "string" },
            "message":{ "type": "string", "maxLength": 50 }
        }
    });

    let response_format_raw = env::var("OPENAI_RESPONSE_FORMAT").ok();
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
        response_format: build_response_format(response_format_raw.as_deref(), schema)?,
    };

    let client = reqwest::Client::new();
    let mut req_builder = client.post(format!("{base}/chat/completions"));
    if let Some(key) = api_key {
        req_builder = req_builder.bearer_auth(key);
    }
    let resp = req_builder
        .json(&req)
        .send()
        .await
        .context("LLM request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "LLM request failed with status {}: {}",
            status,
            text
        ));
    }

    let parsed: ChatResponse = resp.json().await.context("failed to parse LLM response")?;

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

    let mut push_cmd = Command::new("git");
    push_cmd.arg("push");
    if !has_upstream() {
        let branch = current_branch()?;
        eprintln!("No upstream set; pushing with `--set-upstream origin {branch}`");
        push_cmd.args(["--set-upstream", "origin", &branch]);
    }
    let status = push_cmd.status().context("failed to run `git push`")?;

    if !status.success() {
        return Err(anyhow!("git push failed with status: {status}"));
    }

    eprintln!("Changes pushed successfully!");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_schema() -> serde_json::Value {
        serde_json::json!({})
    }

    // ---------- build_response_format ----------

    #[test]
    fn response_format_default_is_json_object() {
        let f = build_response_format(None, empty_schema()).unwrap();
        assert!(matches!(f, Some(ResponseFormat::JsonObject)));
    }

    #[test]
    fn response_format_explicit_json_object() {
        let f = build_response_format(Some("json_object"), empty_schema()).unwrap();
        assert!(matches!(f, Some(ResponseFormat::JsonObject)));
    }

    #[test]
    fn response_format_json_schema_has_strict_and_name() {
        let f = build_response_format(Some("json_schema"), empty_schema()).unwrap();
        match f {
            Some(ResponseFormat::JsonSchema { json_schema }) => {
                assert_eq!(json_schema.name, "commit_message");
                assert!(json_schema.strict);
            }
            other => panic!("expected JsonSchema variant, got {other:?}"),
        }
    }

    #[test]
    fn response_format_none_returns_no_payload() {
        let f = build_response_format(Some("none"), empty_schema()).unwrap();
        assert!(f.is_none());
    }

    #[test]
    fn response_format_is_case_insensitive_and_trimmed() {
        let f = build_response_format(Some("  JSON_Object  "), empty_schema()).unwrap();
        assert!(matches!(f, Some(ResponseFormat::JsonObject)));
    }

    #[test]
    fn response_format_unknown_value_lists_valid_choices() {
        let err = build_response_format(Some("garbage"), empty_schema()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("json_object"), "msg: {msg}");
        assert!(msg.contains("json_schema"), "msg: {msg}");
        assert!(msg.contains("none"), "msg: {msg}");
        assert!(msg.contains("garbage"), "msg: {msg}");
    }

    // ---------- ResponseFormat wire format (regression guard for the enum tag) ----------

    #[test]
    fn json_object_serializes_with_type_only() {
        let v = serde_json::to_value(ResponseFormat::JsonObject).unwrap();
        assert_eq!(v, serde_json::json!({ "type": "json_object" }));
    }

    #[test]
    fn json_schema_serializes_with_nested_schema() {
        let rf = ResponseFormat::JsonSchema {
            json_schema: JsonSchema {
                name: "commit_message".into(),
                schema: serde_json::json!({ "type": "object" }),
                strict: true,
            },
        };
        assert_eq!(
            serde_json::to_value(rf).unwrap(),
            serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "commit_message",
                    "schema": { "type": "object" },
                    "strict": true,
                }
            })
        );
    }

    // ---------- build_commit_line ----------

    #[test]
    fn commit_line_with_scope() {
        let c = Commit {
            r#type: "feat".into(),
            scope: "auth".into(),
            message: "add login".into(),
        };
        assert_eq!(build_commit_line(&c), "feat(auth): add login");
    }

    #[test]
    fn commit_line_without_scope() {
        let c = Commit {
            r#type: "fix".into(),
            scope: "".into(),
            message: "off-by-one".into(),
        };
        assert_eq!(build_commit_line(&c), "fix: off-by-one");
    }

    #[test]
    fn commit_line_drops_whitespace_only_scope() {
        let c = Commit {
            r#type: "chore".into(),
            scope: "   ".into(),
            message: "tidy".into(),
        };
        assert_eq!(build_commit_line(&c), "chore: tidy");
    }

    #[test]
    fn commit_line_trims_all_fields() {
        let c = Commit {
            r#type: "  docs  ".into(),
            scope: "  readme  ".into(),
            message: "  fix typo  ".into(),
        };
        assert_eq!(build_commit_line(&c), "docs(readme): fix typo");
    }

    // ---------- Commit deserialization (model output parsing) ----------

    #[test]
    fn commit_parses_full_json() {
        let c: Commit =
            serde_json::from_str(r#"{"type":"feat","scope":"api","message":"add endpoint"}"#)
                .unwrap();
        assert_eq!(c.r#type, "feat");
        assert_eq!(c.scope, "api");
        assert_eq!(c.message, "add endpoint");
    }

    #[test]
    fn commit_missing_scope_defaults_to_empty() {
        let c: Commit = serde_json::from_str(r#"{"type":"fix","message":"x"}"#).unwrap();
        assert_eq!(c.scope, "");
    }
}
