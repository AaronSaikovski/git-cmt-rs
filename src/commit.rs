//! Commit data, tolerant model-output parsing, and message formatting.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// Defense in depth for direct callers, matching the HTTP response-body limit.
const MAX_MODEL_CONTENT_BYTES: usize = 1024 * 1024;

/// Fields parsed from a model response to form a conventional commit subject.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Commit {
    /// Conventional commit category, such as `feat`, `fix`, or `docs`.
    #[serde(default)]
    pub(crate) r#type: String,
    /// Optional component name; an empty or whitespace-only scope is omitted.
    #[serde(default)]
    pub(crate) scope: String,
    /// Subject text; the model is prompted to keep this within 50 characters.
    #[serde(default)]
    pub(crate) message: String,
}

/// Parses model output as JSON, then tolerates decorated keys and embedded JSON.
///
/// # Errors
///
/// Returns an error if the input exceeds 1 MiB or no commit with a nonblank type
/// or message can be recovered. Embedded prose and Markdown fences are accepted.
pub(crate) fn parse_commit(content: &str) -> Result<Commit> {
    if content.len() > MAX_MODEL_CONTENT_BYTES {
        return Err(anyhow!(
            "model content exceeds the {MAX_MODEL_CONTENT_BYTES}-byte limit"
        ));
    }

    let trimmed = content.trim();

    // Fast path: strict `{ "type", "scope", "message" }` object. Require a
    // meaningful field so odd-keyed objects (e.g. `{"_type": ...}`) that
    // deserialize into an all-empty `Commit` fall through to coercion below.
    if let Ok(commit) = serde_json::from_str::<Commit>(trimmed)
        && (!commit.r#type.trim().is_empty() || !commit.message.trim().is_empty())
    {
        return Ok(commit);
    }

    // Lenient path: parse as generic JSON and coerce. Handles objects with
    // decorated keys (`_type`) and the flattened `[key, value, ...]` arrays
    // some local models emit instead of an object.
    if let Ok(value) = serde_json::from_str::<Value>(trimmed)
        && let Some(commit) = commit_from_value(&value)
    {
        return Ok(commit);
    }

    // Last resort: scan for the first balanced JSON fragment embedded in prose
    // or markdown fences, then coerce it.
    if let Some(fragment) = extract_json_fragment(content)
        && let Ok(value) = serde_json::from_str::<Value>(fragment)
        && let Some(commit) = commit_from_value(&value)
    {
        return Ok(commit);
    }

    Err(anyhow!("failed to parse commit JSON (raw: {content:?})"))
}

// Coerce objects, arrays of candidates, or flattened `[key, value, ...]`
// arrays into a commit. Decorated keys such as `_type` are accepted.
fn commit_from_value(value: &Value) -> Option<Commit> {
    let mut commit = Commit::default();
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                assign_commit_field(&mut commit, key, value);
            }
        }
        // An array containing objects is a list of commit candidates (some
        // models emit one per file). Use the first commit-shaped candidate.
        Value::Array(items) if items.iter().any(Value::is_object) => {
            return items.iter().find_map(commit_from_value);
        }
        // Ignore a dangling trailing key with no value.
        Value::Array(items) => {
            let (pairs, _) = items.as_chunks::<2>();
            for [key, value] in pairs {
                match key {
                    Value::String(key) => assign_commit_field(&mut commit, key, value),
                    key => assign_commit_field(&mut commit, &value_to_string(key), value),
                }
            }
        }
        _ => return None,
    }

    if !commit.r#type.trim().is_empty() || !commit.message.trim().is_empty() {
        Some(commit)
    } else {
        None
    }
}

fn assign_commit_field(commit: &mut Commit, key: &str, value: &Value) {
    let field = match normalize_key(key).as_str() {
        "type" => &mut commit.r#type,
        "scope" => &mut commit.scope,
        "message" => &mut commit.message,
        _ => return,
    };
    *field = value_to_string(value);
}

// Reduce a key to its bare alphabetic identifier so decorated variants such as
// `_type`, `Type ` or `"scope"` all normalize to the canonical field name.
#[must_use]
fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(char::is_ascii_alphabetic)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

// Render a JSON scalar as a plain string; strings are unquoted, null is empty,
// and numbers/bools use their JSON text.
#[must_use]
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

// Find the earliest balanced fragment without rescanning suffixes. Each closer
// is a node in a persistent stack: skipping a quoted string restores the stack
// snapshot after its closing quote. Separate stacks preserve the tolerant
// object/array matching, including recovery inside unmatched outer fragments.
// Every byte and backslash run is visited at most twice; storage is linear in
// the number of closers and bounded by MAX_MODEL_CONTENT_BYTES at the caller.
fn extract_json_fragment(s: &str) -> Option<&str> {
    const NONE: usize = usize::MAX;
    struct ClosingDelimiter {
        index: usize,
        next: usize,
    }

    let bytes = s.as_bytes();
    let mut closers: Vec<ClosingDelimiter> = Vec::new();
    let mut next_close = [NONE; 2];
    let mut after_quote = [NONE; 2];
    let mut fragment = None;

    let reversed_bytes = bytes.iter().enumerate().rev();
    for (index, &byte) in reversed_bytes {
        match byte {
            b'}' | b']' => {
                let kind = usize::from(byte == b']');
                closers.push(ClosingDelimiter {
                    index,
                    next: next_close[kind],
                });
                next_close[kind] = closers.len() - 1;
            }
            b'{' | b'[' => {
                let kind = usize::from(byte == b'[');
                if next_close[kind] != NONE {
                    let closer = &closers[next_close[kind]];
                    fragment = Some(&s[index..=closer.index]);
                    next_close[kind] = closer.next;
                }
            }
            b'"' => {
                let after_current_quote = next_close;
                next_close = after_quote;

                // Outside a string every quote opens one; inside a string,
                // only a quote preceded by an even backslash run closes it.
                let backslashes = bytes[..index]
                    .iter()
                    .rev()
                    .take_while(|&&byte| byte == b'\\')
                    .count();
                if backslashes % 2 == 0 {
                    after_quote = after_current_quote;
                }
            }
            _ => {}
        }
    }

    fragment
}

/// Formats a commit subject, trimming fields and omitting an empty scope.
#[must_use]
pub(crate) fn build_commit_line(commit: &Commit) -> String {
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

#[cfg(test)]
mod tests {
    use super::{Commit, MAX_MODEL_CONTENT_BYTES};
    use std::iter;

    // ---------- build_commit_line ----------

    #[test]
    fn commit_line_with_scope() {
        let c = Commit {
            r#type: "feat".into(),
            scope: "auth".into(),
            message: "add login".into(),
        };
        assert_eq!(super::build_commit_line(&c), "feat(auth): add login");
    }

    #[test]
    fn commit_line_without_scope() {
        let c = Commit {
            r#type: "fix".into(),
            scope: "".into(),
            message: "off-by-one".into(),
        };
        assert_eq!(super::build_commit_line(&c), "fix: off-by-one");
    }

    #[test]
    fn commit_line_drops_whitespace_only_scope() {
        let c = Commit {
            r#type: "chore".into(),
            scope: "   ".into(),
            message: "tidy".into(),
        };
        assert_eq!(super::build_commit_line(&c), "chore: tidy");
    }

    #[test]
    fn commit_line_trims_all_fields() {
        let c = Commit {
            r#type: "  docs  ".into(),
            scope: "  readme  ".into(),
            message: "  fix typo  ".into(),
        };
        assert_eq!(super::build_commit_line(&c), "docs(readme): fix typo");
    }

    // ---------- Commit deserialization (model output parsing) ----------

    #[test]
    fn commit_parses_full_json() {
        let c: Commit =
            serde_json::from_str(r#"{"type":"feat","scope":"api","message":"add endpoint"}"#)
                .expect("complete commit JSON should deserialize");
        assert_eq!(c.r#type, "feat");
        assert_eq!(c.scope, "api");
        assert_eq!(c.message, "add endpoint");
    }

    #[test]
    fn commit_missing_scope_defaults_to_empty() {
        let c: Commit = serde_json::from_str(r#"{"type":"fix","message":"x"}"#)
            .expect("an omitted scope should default to an empty string");
        assert_eq!(c.scope, "");
    }

    // ---------- parse_commit (lenient model-output parsing) ----------

    #[test]
    fn parse_commit_accepts_plain_json() {
        let c = super::parse_commit(r#"{"type":"feat","scope":"api","message":"add endpoint"}"#)
            .expect("plain commit JSON should parse");
        assert_eq!(c.r#type, "feat");
        assert_eq!(c.scope, "api");
    }

    #[test]
    fn parse_commit_strips_markdown_fences() {
        let raw = "```json\n{\n  \"type\": \"chore\",\n  \"scope\": \"deps\",\n  \"message\": \"bump\"\n}\n```";
        let c = super::parse_commit(raw).expect("Markdown fences should not obscure commit JSON");
        assert_eq!(c.r#type, "chore");
        assert_eq!(c.scope, "deps");
        assert_eq!(c.message, "bump");
    }

    #[test]
    fn parse_commit_handles_gemma_stray_brace_and_double_fence() {
        // Exact shape emitted by a Gemma model via Ollama: a `json` label, a
        // stray unclosed `{`, then a fenced object.
        let raw = "json\n{```json\n{\n  \"type\": \"chore\",\n  \"scope\": \"deps\",\n  \"message\": \"Update dependencies\"\n}\n```";
        let c = super::parse_commit(raw)
            .expect("a stray opening brace should not obscure the fenced commit");
        assert_eq!(c.r#type, "chore");
        assert_eq!(c.scope, "deps");
        assert_eq!(c.message, "Update dependencies");
    }

    #[test]
    fn parse_commit_preserves_braces_inside_message() {
        let raw = "sure!\n{\"type\":\"fix\",\"scope\":\"\",\"message\":\"handle {x} token\"}";
        let c =
            super::parse_commit(raw).expect("message braces should remain inside the JSON string");
        assert_eq!(c.message, "handle {x} token");
    }

    #[test]
    fn parse_commit_errors_on_no_json() {
        super::parse_commit("I could not generate a commit message.")
            .expect_err("prose without JSON should not produce a commit");
    }

    #[test]
    fn parse_commit_handles_flattened_key_value_array() {
        // Exact shape emitted by a local model: an object flattened into a
        // `[key, value, ...]` array with underscore-decorated keys.
        let raw = r#"["_type", "chore", "_scope", "dependencies", "_message", "Add Microsoft.OpenApi package to pin version for security fix."]"#;
        let c =
            super::parse_commit(raw).expect("decorated keys in alternating arrays should parse");
        assert_eq!(c.r#type, "chore");
        assert_eq!(c.scope, "dependencies");
        assert_eq!(
            c.message,
            "Add Microsoft.OpenApi package to pin version for security fix."
        );
    }

    #[test]
    fn parse_commit_handles_underscore_keyed_object() {
        let raw = r#"{"_type": "feat", "_scope": "api", "_message": "add endpoint"}"#;
        let c = super::parse_commit(raw).expect("underscore-decorated object keys should parse");
        assert_eq!(c.r#type, "feat");
        assert_eq!(c.scope, "api");
        assert_eq!(c.message, "add endpoint");
    }

    #[test]
    fn parse_commit_extracts_flattened_array_from_prose() {
        let raw = "Here you go:\n[\"type\", \"fix\", \"scope\", \"\", \"message\", \"off-by-one\"]";
        let c =
            super::parse_commit(raw).expect("alternating arrays embedded in prose should parse");
        assert_eq!(c.r#type, "fix");
        assert_eq!(c.scope, "");
        assert_eq!(c.message, "off-by-one");
    }

    #[test]
    fn parse_commit_reads_three_element_array_positionally() {
        // serde deserializes a struct from a 3-element sequence positionally
        // (type, scope, message), which is a reasonable interpretation.
        let c = super::parse_commit(r#"["fix", "api", "off-by-one"]"#)
            .expect("three-element string arrays should deserialize positionally");
        assert_eq!(c.r#type, "fix");
        assert_eq!(c.scope, "api");
        assert_eq!(c.message, "off-by-one");
    }

    #[test]
    fn parse_commit_rejects_array_without_commit_keys() {
        super::parse_commit(r#"["a", "b", "c", "d"]"#)
            .expect_err("an alternating array without commit keys should be rejected");
    }

    #[test]
    fn parse_commit_handles_odd_length_flattened_array() {
        // A flattened array with a dangling trailing key (no value). Pair what
        // we can and drop the orphan rather than failing.
        let raw = r#"["_type", "chore", "_scope", "deps", "message"]"#;
        let c = super::parse_commit(raw).expect("a dangling array key should be ignored");
        assert_eq!(c.r#type, "chore");
        assert_eq!(c.scope, "deps");
        assert_eq!(c.message, "");
    }

    #[test]
    fn parse_commit_reads_first_of_array_of_objects() {
        // Some models emit one commit object per changed file. Take the first.
        let raw = r#"[{"type": "docs", "scope": ".github/workflows/api-deploy-reusable.yml", "message": "Remove Stripe price ID config from API deploy workflow"}, {"type": "docs", "scope": ".github/workflows/infra-deploy-reusable.yml", "message": "Remove Stripe price ID handling from infra deploy workflow"}]"#;
        let c =
            super::parse_commit(raw).expect("an array of objects should yield its first commit");
        assert_eq!(c.r#type, "docs");
        assert_eq!(c.scope, ".github/workflows/api-deploy-reusable.yml");
        assert_eq!(
            c.message,
            "Remove Stripe price ID config from API deploy workflow"
        );
    }

    #[test]
    fn parse_commit_coerces_nonstring_object_fields() {
        let raw = r#"{"_type":42,"_scope":null,"_message":true,"ignored":{"nested":"value"}}"#;
        let commit = super::parse_commit(raw)
            .expect("recognized fields should preserve scalar coercion and ignore unknown fields");
        assert_eq!(commit.r#type, "42");
        assert_eq!(commit.scope, "");
        assert_eq!(commit.message, "true");
    }

    #[test]
    fn parse_commit_preserves_alternating_array_coercion_and_overrides() {
        let raw = r#"["type","fix","type","feat",["message"],["payload"],"dangling"]"#;
        let commit = super::parse_commit(raw)
            .expect("alternating arrays should coerce nonstring keys and values");
        assert_eq!(commit.r#type, "feat");
        assert_eq!(commit.message, r#"["payload"]"#);
    }

    #[test]
    fn parse_commit_skips_invalid_candidates_before_alternating_array() {
        let raw = r#"[null,{"scope":"ignored"},["type","fix","message","candidate"],{"type":"feat","message":"later"}]"#;
        let commit = super::parse_commit(raw)
            .expect("candidate lists should accept the first commit-shaped value");
        assert_eq!(super::build_commit_line(&commit), "fix: candidate");
    }

    #[test]
    fn parse_commit_rejects_long_unmatched_delimiters() {
        for opener in ['{', '['] {
            let raw = opener.to_string().repeat(128 * 1024);
            super::parse_commit(&raw)
                .expect_err("unmatched delimiters without commit fields should be rejected");
        }
    }

    #[test]
    fn parse_commit_recovers_nested_fragment_after_many_stray_openers() {
        for prefix in ["{", "[", "[{"] {
            let raw = format!(
                "{}\n```json\n{{\"type\":\"fix\",\"message\":\"recover\"}}\n```",
                prefix.repeat(32 * 1024)
            );
            let commit = super::parse_commit(&raw)
                .expect("many stray openers should not obscure a nested commit");
            assert_eq!(super::build_commit_line(&commit), "fix: recover");
        }
    }

    #[test]
    fn parse_commit_recovers_fragment_after_unterminated_string() {
        let raw = r#"prose {"unfinished { "type":"fix","message":"recover" }"#;
        let commit = super::parse_commit(raw)
            .expect("an unterminated outer string should not obscure a nested commit");
        assert_eq!(super::build_commit_line(&commit), "fix: recover");
    }

    #[test]
    fn parse_commit_extracts_mixed_nesting_and_escaped_strings() {
        let raw = r#"Here's "the result":
```json
[{"ignored":{"items":[1,{"text":"quote: \" slash: \\ brackets: [{}]"}]},"type":"fix","scope":"parser","message":"keep \"quotes\", \\ and [{tokens}]"}]
```"#;
        let commit = super::parse_commit(raw)
            .expect("nested JSON and escaped string delimiters should parse");
        assert_eq!(commit.r#type, "fix");
        assert_eq!(commit.scope, "parser");
        assert_eq!(commit.message, r#"keep "quotes", \ and [{tokens}]"#);
    }

    #[test]
    fn parse_commit_prefers_outer_fragment_over_completed_inner_fragment() {
        let raw =
            r#"prose {"type":"feat","message":"outer","nested":{"type":"fix","message":"inner"}}"#;
        let commit = super::parse_commit(raw).expect("a complete outer commit should parse");
        assert_eq!(super::build_commit_line(&commit), "feat: outer");
    }

    #[test]
    fn parse_commit_accepts_model_content_at_limit() {
        let mut raw = r#"{"type":"fix","message":"at limit"}"#.to_owned();
        raw.extend(iter::repeat_n(' ', MAX_MODEL_CONTENT_BYTES - raw.len()));
        let commit = super::parse_commit(&raw)
            .expect("model content at the byte limit should still be accepted");
        assert_eq!(super::build_commit_line(&commit), "fix: at limit");
    }

    #[test]
    fn parse_commit_rejects_oversize_model_content_before_trimming() {
        let mut raw = r#"{"type":"fix","message":"too large"}"#.to_owned();
        raw.extend(iter::repeat_n(' ', MAX_MODEL_CONTENT_BYTES + 1 - raw.len()));
        let error = super::parse_commit(&raw)
            .expect_err("oversize model content should be rejected before trimming");
        assert_eq!(
            error.to_string(),
            "model content exceeds the 1048576-byte limit"
        );
    }

    #[test]
    fn parse_commit_extracts_string_ending_in_escaped_backslash() {
        let raw = r#"prose {"type":"fix","message":"path\\"} trailing"#;
        let commit = super::parse_commit(raw)
            .expect("an escaped trailing backslash should not hide the closing quote");
        assert_eq!(commit.message, "path\\");
    }

    #[test]
    fn parse_commit_extracts_deep_json_supported_by_serde() {
        let raw = format!(
            "prose {{\"ignored\":{}0{},\"type\":\"fix\",\"message\":\"deep\"}}",
            "[".repeat(120),
            "]".repeat(120)
        );
        let commit =
            super::parse_commit(&raw).expect("deep JSON within serde's nesting limit should parse");
        assert_eq!(super::build_commit_line(&commit), "fix: deep");
    }
}
