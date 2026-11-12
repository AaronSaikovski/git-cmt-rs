//! OpenAI-compatible request construction and message generation.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::env;

use crate::commit::{Commit, parse_commit};

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
fn build_chat_request(
    base: &str,
    api_key: Option<&str>,
    request: &ChatRequest,
) -> reqwest::RequestBuilder {
    let client = reqwest::Client::new();
    let mut builder = client.post(format!("{base}/chat/completions"));
    // Local backends may reject even an empty Authorization header.
    if let Some(key) = api_key.filter(|key| !key.is_empty()) {
        builder = builder.bearer_auth(key);
    }
    builder.json(request)
}

pub(crate) async fn generate_message(changes: &str) -> Result<Commit> {
    let api_key = env::var("OPENAI_API_KEY").ok();
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

    let resp = build_chat_request(&base, api_key.as_deref(), &req)
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

    // Model should have returned strict JSON per schema, but local models
    // (e.g. Gemma via Ollama) often wrap it in markdown fences or stray text.
    parse_commit(&content)
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

    // ---------- Chat Completions request compatibility ----------

    fn backend_request(api_key: Option<&str>) -> reqwest::Request {
        let request = ChatRequest {
            model: "qwen/qwen3.6-27b".into(),
            messages: vec![Message {
                role: "user".into(),
                content: "Changes:\n+installation instructions".into(),
            }],
            temperature: 0.0,
            response_format: build_response_format(Some("none"), empty_schema()).unwrap(),
        };
        build_chat_request("http://localhost:1235/v1", api_key, &request)
            .build()
            .unwrap()
    }

    #[test]
    fn backend_request_without_api_key_omits_authorization() {
        let request = backend_request(None);
        assert!(
            !request
                .headers()
                .contains_key(reqwest::header::AUTHORIZATION)
        );
    }

    #[test]
    fn backend_request_with_empty_api_key_omits_authorization() {
        let request = backend_request(Some(""));
        assert!(
            !request
                .headers()
                .contains_key(reqwest::header::AUTHORIZATION)
        );
    }

    #[test]
    fn backend_request_with_api_key_sends_bearer_authorization() {
        let request = backend_request(Some("test-api-key"));
        assert_eq!(
            request.headers()[reqwest::header::AUTHORIZATION],
            "Bearer test-api-key"
        );
    }

    #[test]
    fn backend_request_for_lm_studio_omits_response_format() {
        let request = backend_request(None);
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "http://localhost:1235/v1/chat/completions"
        );
        assert_eq!(
            request.headers()[reqwest::header::CONTENT_TYPE],
            "application/json"
        );
        let body: serde_json::Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert!(body.get("response_format").is_none(), "{body}");
        assert_eq!(body["model"], "qwen/qwen3.6-27b");
        assert_eq!(
            body["messages"][0],
            serde_json::json!({
                "role": "user",
                "content": "Changes:\n+installation instructions"
            })
        );
    }
}
