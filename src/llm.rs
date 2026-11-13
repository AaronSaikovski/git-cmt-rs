//! OpenAI-compatible request construction and message generation.

use anyhow::{Context, Result, anyhow};
use reqwest::{Client, ClientBuilder, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::time::Duration;

use crate::commit;
use crate::commit::Commit;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

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
    schema: Value,
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
fn build_response_format(raw: Option<&str>, schema: Value) -> Result<Option<ResponseFormat>> {
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

fn http_client_builder(request_timeout: Duration) -> ClientBuilder {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        // Reqwest applies this deadline through consumption of the response body.
        .timeout(request_timeout)
}

fn build_chat_request(
    client: &Client,
    base: &str,
    api_key: Option<&str>,
    request: &ChatRequest,
) -> RequestBuilder {
    let mut builder = client.post(format!("{base}/chat/completions"));
    // Local backends may reject even an empty Authorization header.
    if let Some(key) = api_key.filter(|key| !key.is_empty()) {
        builder = builder.bearer_auth(key);
    }
    builder.json(request)
}

async fn request_chat(
    client: &Client,
    base: &str,
    api_key: Option<&str>,
    request: &ChatRequest,
) -> Result<ChatResponse> {
    let mut response = build_chat_request(client, base, api_key, request)
        .send()
        .await
        .context("LLM request failed")?;
    let status = response.status();
    let mut body = Vec::new();
    // Chunks are decoded by reqwest, so compressed and chunked responses obey
    // the same limit even when Content-Length is missing or misleading.
    while let Some(chunk) = response
        .chunk()
        .await
        .with_context(|| format!("failed to read LLM response with status {status}"))?
    {
        let remaining = MAX_RESPONSE_BYTES - body.len();
        if chunk.len() > remaining {
            if !status.is_success() {
                body.extend_from_slice(&chunk[..remaining]);
                return Err(anyhow!(
                    "LLM request failed with status {status}: response exceeds {MAX_RESPONSE_BYTES} bytes (body truncated): {}",
                    String::from_utf8_lossy(&body)
                ));
            }
            return Err(anyhow!(
                "LLM response with status {status} exceeds {MAX_RESPONSE_BYTES} bytes"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        return Err(anyhow!(
            "LLM request failed with status {status}: {}",
            String::from_utf8_lossy(&body)
        ));
    }
    serde_json::from_slice(&body).context("failed to parse LLM response")
}

/// Generates a commit from the supplied diff using the configured OpenAI-compatible backend.
///
/// Returns an error for invalid configuration, failed or oversized HTTP responses,
/// or a response that cannot be parsed into a commit.
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
    let request = ChatRequest {
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

    let client = http_client_builder(REQUEST_TIMEOUT)
        .build()
        .context("failed to build LLM HTTP client")?;
    let parsed = request_chat(&client, &base, api_key.as_deref(), &request).await?;

    let content = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no choices returned"))?
        .message
        .content;

    // Model should have returned strict JSON per schema, but local models
    // (e.g. Gemma via Ollama) often wrap it in markdown fences or stray text.
    commit::parse_commit(&content)
}

#[cfg(test)]
mod tests {
    use super::{ChatRequest, JsonSchema, Message, REQUEST_TIMEOUT, ResponseFormat};
    use reqwest::{Method, Request};
    use serde_json::Value;

    fn empty_schema() -> Value {
        serde_json::json!({})
    }

    // ---------- build_response_format ----------

    #[test]
    fn response_format_default_is_json_object() {
        let format = super::build_response_format(None, empty_schema())
            .expect("default response format should be supported");
        assert!(matches!(format, Some(ResponseFormat::JsonObject)));
    }

    #[test]
    fn response_format_explicit_json_object() {
        let format = super::build_response_format(Some("json_object"), empty_schema())
            .expect("json_object response format should be supported");
        assert!(matches!(format, Some(ResponseFormat::JsonObject)));
    }

    #[test]
    fn response_format_json_schema_has_strict_and_name() {
        let format = super::build_response_format(Some("json_schema"), empty_schema())
            .expect("json_schema response format should be supported");
        match format {
            Some(ResponseFormat::JsonSchema { json_schema }) => {
                assert_eq!(json_schema.name, "commit_message");
                assert!(json_schema.strict);
            }
            other => panic!("expected JsonSchema variant, got {other:?}"),
        }
    }

    #[test]
    fn response_format_none_returns_no_payload() {
        let format = super::build_response_format(Some("none"), empty_schema())
            .expect("none response format should be supported");
        assert!(format.is_none());
    }

    #[test]
    fn response_format_is_case_insensitive_and_trimmed() {
        let format = super::build_response_format(Some("  JSON_Object  "), empty_schema())
            .expect("response format should accept surrounding whitespace and mixed case");
        assert!(matches!(format, Some(ResponseFormat::JsonObject)));
    }

    #[test]
    fn response_format_unknown_value_lists_valid_choices() {
        let error = super::build_response_format(Some("garbage"), empty_schema())
            .expect_err("unknown response formats should be rejected");
        let message = error.to_string();
        assert!(message.contains("json_object"), "{message}");
        assert!(message.contains("json_schema"), "{message}");
        assert!(message.contains("none"), "{message}");
        assert!(message.contains("garbage"), "{message}");
    }

    // ---------- ResponseFormat wire format (regression guard for the enum tag) ----------

    #[test]
    fn json_object_serializes_with_type_only() {
        let value = serde_json::to_value(ResponseFormat::JsonObject)
            .expect("json_object format should serialize");
        assert_eq!(value, serde_json::json!({ "type": "json_object" }));
    }

    #[test]
    fn json_schema_serializes_with_nested_schema() {
        let format = ResponseFormat::JsonSchema {
            json_schema: JsonSchema {
                name: "commit_message".into(),
                schema: serde_json::json!({ "type": "object" }),
                strict: true,
            },
        };
        assert_eq!(
            serde_json::to_value(format).expect("json_schema format should serialize"),
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

    fn backend_request(api_key: Option<&str>) -> Request {
        let request = ChatRequest {
            model: "qwen/qwen3.6-27b".into(),
            messages: vec![Message {
                role: "user".into(),
                content: "Changes:\n+installation instructions".into(),
            }],
            temperature: 0.0,
            response_format: super::build_response_format(Some("none"), empty_schema())
                .expect("none response format should be supported"),
        };
        let client = super::http_client_builder(REQUEST_TIMEOUT)
            .build()
            .expect("test HTTP client should build");
        super::build_chat_request(&client, "http://localhost:1235/v1", api_key, &request)
            .build()
            .expect("test chat request should build")
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
        assert_eq!(request.method(), Method::POST);
        assert_eq!(
            request.url().as_str(),
            "http://localhost:1235/v1/chat/completions"
        );
        assert_eq!(
            request.headers()[reqwest::header::CONTENT_TYPE],
            "application/json"
        );
        let body: Value = serde_json::from_slice(
            request
                .body()
                .expect("chat request should have a JSON body")
                .as_bytes()
                .expect("JSON request body should be buffered"),
        )
        .expect("chat request body should contain valid JSON");
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

#[cfg(test)]
mod http_tests {
    use super::{ChatRequest, ChatResponse, MAX_RESPONSE_BYTES};
    use anyhow::Result;
    use std::io::{BufReader, ErrorKind};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::sync::mpsc::{RecvTimeoutError, Sender};
    use std::thread;
    use std::thread::JoinHandle;
    use std::time::Duration;

    struct TestServer {
        base: String,
        release: Sender<()>,
        thread: Option<JoinHandle<()>>,
    }

    impl TestServer {
        fn new(response: Vec<u8>) -> Self {
            use std::io::{BufRead, Read, Write};

            let listener = TcpListener::bind("127.0.0.1:0")
                .expect("test server should bind an available loopback port");
            listener
                .set_nonblocking(true)
                .expect("test listener should support nonblocking accepts");
            let base = format!(
                "http://{}",
                listener
                    .local_addr()
                    .expect("test listener should have an address")
            );
            let (release, released) = mpsc::channel();
            let thread = thread::spawn(move || {
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            match released.recv_timeout(Duration::from_millis(10)) {
                                Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
                                Err(RecvTimeoutError::Timeout) => {}
                            }
                        }
                        Err(error) => panic!("failed to accept test request: {error}"),
                    }
                };
                socket
                    .set_nonblocking(false)
                    .expect("accepted test socket should support blocking reads");
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("test socket should accept a read timeout");
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .expect("test socket should accept a write timeout");
                let mut reader = BufReader::new(&mut socket);
                let mut content_length = 0;
                loop {
                    let mut line = String::new();
                    assert_ne!(
                        reader
                            .read_line(&mut line)
                            .expect("request header should be readable"),
                        0,
                        "client should send complete request headers"
                    );
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        content_length = value
                            .trim()
                            .parse()
                            .expect("request content length should be an integer");
                    }
                }
                let mut request_body = vec![0; content_length];
                reader
                    .read_exact(&mut request_body)
                    .expect("client should send the complete request body");
                drop(reader);
                // Oversized responses may be rejected before the last write.
                if let Err(error) = socket.write_all(&response) {
                    assert!(matches!(
                        error.kind(),
                        ErrorKind::BrokenPipe | ErrorKind::ConnectionReset
                    ));
                }
                // Keep the connection open until the client has returned. This
                // makes stalls deterministic without sleeps or elapsed checks.
                let _ = released.recv();
            });
            Self {
                base,
                release,
                thread: Some(thread),
            }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            let _ = self.release.send(());
            self.thread
                .take()
                .expect("test server should own its worker thread")
                .join()
                .expect("test server worker should finish without panicking");
        }
    }

    async fn fetch(response: Vec<u8>, timeout: Duration) -> Result<ChatResponse> {
        let server = TestServer::new(response);
        let client = super::http_client_builder(timeout)
            .no_proxy()
            .build()
            .expect("test HTTP client should build");
        let request = ChatRequest {
            model: "test".into(),
            messages: vec![],
            temperature: 0.0,
            response_format: None,
        };
        super::request_chat(&client, &server.base, None, &request).await
    }

    fn chunked_response(status: &str, body: &[u8]) -> Vec<u8> {
        use std::io::Write;

        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n"
        )
        .into_bytes();
        for chunk in body.chunks(8192) {
            write!(response, "{:X}\r\n", chunk.len())
                .expect("chunk header should write to the response buffer");
            response.extend_from_slice(chunk);
            response.extend_from_slice(b"\r\n");
        }
        response.extend_from_slice(b"0\r\n\r\n");
        response
    }

    #[tokio::test]
    async fn chunked_success_over_limit_is_rejected() {
        let response = chunked_response("200 OK", &vec![b'x'; MAX_RESPONSE_BYTES + 1]);
        let error = fetch(response, Duration::from_secs(5))
            .await
            .expect_err("oversized chunked success body should be rejected");
        let message = error.to_string();
        assert!(message.contains("200 OK"), "{message}");
        assert!(message.contains("exceeds 1048576 bytes"), "{message}");
    }

    #[tokio::test]
    async fn chunked_error_over_limit_is_rejected_with_status_and_bounded_body() {
        let response = chunked_response(
            "503 Service Unavailable",
            &vec![b'x'; MAX_RESPONSE_BYTES + 1],
        );
        let error = fetch(response, Duration::from_secs(5))
            .await
            .expect_err("oversized chunked error body should be rejected");
        let message = error.to_string();
        let (context, body) = message
            .split_once("(body truncated): ")
            .expect("oversized error should include its truncated body");
        assert!(context.contains("503 Service Unavailable"));
        assert!(context.contains("exceeds 1048576 bytes"));
        assert_eq!(body.len(), MAX_RESPONSE_BYTES);
        assert!(body.bytes().all(|byte| byte == b'x'));
    }

    async fn assert_unframed_body_is_bounded(status: &str) {
        let mut response = format!("HTTP/1.1 {status}\r\n\r\n").into_bytes();
        response.resize(response.len() + MAX_RESPONSE_BYTES + 1, b'x');
        let error = fetch(response, Duration::from_secs(5))
            .await
            .expect_err("oversized unframed body should be rejected before EOF");
        let message = error.to_string();
        assert!(message.contains(status));
        assert!(message.contains("exceeds 1048576 bytes"));
    }

    #[tokio::test]
    async fn success_without_content_length_is_bounded_before_eof() {
        assert_unframed_body_is_bounded("200 OK").await;
    }

    #[tokio::test]
    async fn error_without_content_length_is_bounded_before_eof() {
        assert_unframed_body_is_bounded("502 Bad Gateway").await;
    }

    #[tokio::test]
    async fn valid_json_at_response_limit_is_unchanged() {
        let content = r#"{"type":"fix","scope":"http","message":"bound responses"}"#;
        let mut body = serde_json::to_vec(&serde_json::json!({
            "choices": [{"message": {"content": content}}]
        }))
        .expect("valid chat response should serialize");
        body.resize(MAX_RESPONSE_BYTES, b' ');
        let parsed = fetch(chunked_response("200 OK", &body), Duration::from_secs(5))
            .await
            .expect("valid JSON at the response limit should be accepted");
        assert_eq!(parsed.choices.len(), 1);
        assert_eq!(parsed.choices[0].message.content, content);
    }

    #[tokio::test]
    async fn provider_error_preserves_status_and_body() {
        let error = fetch(
            chunked_response("401 Unauthorized", br#"{"error":"invalid API key"}"#),
            Duration::from_secs(5),
        )
        .await
        .expect_err("provider error status should be rejected");
        assert_eq!(
            error.to_string(),
            r#"LLM request failed with status 401 Unauthorized: {"error":"invalid API key"}"#
        );
    }

    fn assert_timeout(error: anyhow::Error, context: &str) {
        assert_eq!(error.to_string(), context);
        assert!(
            error
                .downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_timeout),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn stalled_headers_hit_request_deadline() {
        let error = fetch(Vec::new(), Duration::from_secs(1))
            .await
            .expect_err("stalled response headers should time out");
        assert_timeout(error, "LLM request failed");
    }

    #[tokio::test]
    async fn stalled_body_hits_request_deadline() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{".to_vec();
        let error = fetch(response, Duration::from_secs(1))
            .await
            .expect_err("stalled response body should time out");
        assert_timeout(error, "failed to read LLM response with status 200 OK");
    }

    fn gzip_over_limit_response(status: &str) -> Vec<u8> {
        // Gzip of 1 MiB + 1 ASCII 'x' bytes. Its encoded length is only 1051,
        // so a limit on Content-Length or encoded bytes would miss it.
        let mut compressed = vec![
            31, 139, 8, 0, 0, 0, 0, 0, 0, 255, 237, 193, 49, 1, 0, 0, 0, 194, 160, 218, 139, 111,
            8, 95, 160,
        ];
        compressed.resize(compressed.len() + 1016, 0);
        compressed.extend_from_slice(&[78, 3, 159, 124, 30, 68, 1, 0, 16, 0]);
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n",
            compressed.len()
        )
        .into_bytes();
        response.extend_from_slice(&compressed);
        response
    }

    #[tokio::test]
    async fn compressed_success_is_limited_after_decoding() {
        let error = fetch(gzip_over_limit_response("200 OK"), Duration::from_secs(5))
            .await
            .expect_err("oversized decoded success body should be rejected");
        let message = error.to_string();
        assert!(message.contains("200 OK"), "{message}");
        assert!(message.contains("exceeds 1048576 bytes"), "{message}");
    }

    #[tokio::test]
    async fn compressed_error_is_limited_after_decoding() {
        let error = fetch(
            gzip_over_limit_response("503 Service Unavailable"),
            Duration::from_secs(5),
        )
        .await
        .expect_err("oversized decoded error body should be rejected");
        let message = error.to_string();
        let (context, body) = message
            .split_once("(body truncated): ")
            .expect("oversized decoded error should include its truncated body");
        assert!(context.contains("503 Service Unavailable"));
        assert!(context.contains("exceeds 1048576 bytes"));
        assert_eq!(body.len(), MAX_RESPONSE_BYTES);
        assert!(body.bytes().all(|byte| byte == b'x'));
    }
}
