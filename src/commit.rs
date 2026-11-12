//! Commit data, tolerant model-output parsing, and message formatting.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Commit {
    #[serde(default)]
    pub(crate) r#type: String, // feat, fix, docs, etc.
    #[serde(default)]
    pub(crate) scope: String, // optional component
    #[serde(default)]
    pub(crate) message: String, // 50 chars max per prompt
}

// Parse a `Commit` from raw model output. Tries the text as-is first, then
// falls back to extracting the first balanced JSON object/array embedded in
// surrounding prose / markdown code fences and coercing it into a `Commit`.
pub(crate) fn parse_commit(content: &str) -> Result<Commit> {
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
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(commit) = commit_from_value(&value)
    {
        return Ok(commit);
    }

    // Last resort: scan for the first balanced JSON fragment embedded in prose
    // or markdown fences, then coerce it.
    if let Some(fragment) = extract_json_fragment(content)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(fragment)
        && let Some(commit) = commit_from_value(&value)
    {
        return Ok(commit);
    }

    Err(anyhow!("failed to parse commit JSON (raw: {content:?})"))
}

// Coerce a generic JSON value into a `Commit`. Accepts a JSON object or a
// flattened `[key, value, key, value, ...]` array, tolerating decorated keys
// like `_type`. Returns `None` if no commit-shaped fields are present.
fn commit_from_value(value: &serde_json::Value) -> Option<Commit> {
    let pairs: Vec<(String, String)> = match value {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| (k.clone(), value_to_string(v)))
            .collect(),
        // An array of objects is a list of commit candidates (some models emit
        // one per file). Coerce the first that yields a commit-shaped value.
        serde_json::Value::Array(items) if items.iter().any(serde_json::Value::is_object) => {
            return items.iter().find_map(commit_from_value);
        }
        // Flattened `[key, value, ...]` array. Pair adjacent elements,
        // tolerating a dangling trailing key with no value (odd length).
        serde_json::Value::Array(items) => items
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| (value_to_string(&pair[0]), value_to_string(&pair[1])))
            .collect(),
        _ => return None,
    };

    let mut commit = Commit::default();
    let mut matched = false;
    for (key, val) in pairs {
        match normalize_key(&key).as_str() {
            "type" => {
                commit.r#type = val;
                matched = true;
            }
            "scope" => {
                commit.scope = val;
                matched = true;
            }
            "message" => {
                commit.message = val;
                matched = true;
            }
            _ => {}
        }
    }

    if matched && (!commit.r#type.trim().is_empty() || !commit.message.trim().is_empty()) {
        Some(commit)
    } else {
        None
    }
}

// Reduce a key to its bare alphabetic identifier so decorated variants such as
// `_type`, `Type ` or `"scope"` all normalize to the canonical field name.
fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(char::is_ascii_alphabetic)
        .collect::<String>()
        .to_lowercase()
}

// Render a JSON scalar as a plain string; strings are unquoted, null is empty,
// and numbers/bools use their JSON text.
fn value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

// Find the first balanced JSON object or array in `s`. String and escape aware,
// so delimiters inside JSON string values don't throw off the depth count.
// Skips leading garbage like a stray `{` that never closes.
fn extract_json_fragment(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    for (start, m) in s.match_indices(['{', '[']) {
        let (open, close) = if m == "{" { (b'{', b'}') } else { (b'[', b']') };
        if let Some(end) = match_delimited(bytes, start, open, close) {
            return Some(&s[start..=end]);
        }
    }
    None
}

// Forward-match the delimiter opened at `start`; returns the byte index of the
// matching close delimiter, or `None` if it never closes.
fn match_delimited(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;

    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        if b == b'"' {
            in_str = true;
        } else if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

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
    use super::*;

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

    // ---------- parse_commit (lenient model-output parsing) ----------

    #[test]
    fn parse_commit_accepts_plain_json() {
        let c = parse_commit(r#"{"type":"feat","scope":"api","message":"add endpoint"}"#).unwrap();
        assert_eq!(c.r#type, "feat");
        assert_eq!(c.scope, "api");
    }

    #[test]
    fn parse_commit_strips_markdown_fences() {
        let raw = "```json\n{\n  \"type\": \"chore\",\n  \"scope\": \"deps\",\n  \"message\": \"bump\"\n}\n```";
        let c = parse_commit(raw).unwrap();
        assert_eq!(c.r#type, "chore");
        assert_eq!(c.scope, "deps");
        assert_eq!(c.message, "bump");
    }

    #[test]
    fn parse_commit_handles_gemma_stray_brace_and_double_fence() {
        // Exact shape emitted by a Gemma model via Ollama: a `json` label, a
        // stray unclosed `{`, then a fenced object.
        let raw = "json\n{```json\n{\n  \"type\": \"chore\",\n  \"scope\": \"deps\",\n  \"message\": \"Update dependencies\"\n}\n```";
        let c = parse_commit(raw).unwrap();
        assert_eq!(c.r#type, "chore");
        assert_eq!(c.scope, "deps");
        assert_eq!(c.message, "Update dependencies");
    }

    #[test]
    fn parse_commit_preserves_braces_inside_message() {
        let raw = "sure!\n{\"type\":\"fix\",\"scope\":\"\",\"message\":\"handle {x} token\"}";
        let c = parse_commit(raw).unwrap();
        assert_eq!(c.message, "handle {x} token");
    }

    #[test]
    fn parse_commit_errors_on_no_json() {
        assert!(parse_commit("I could not generate a commit message.").is_err());
    }

    #[test]
    fn parse_commit_handles_flattened_key_value_array() {
        // Exact shape emitted by a local model: an object flattened into a
        // `[key, value, ...]` array with underscore-decorated keys.
        let raw = r#"["_type", "chore", "_scope", "dependencies", "_message", "Add Microsoft.OpenApi package to pin version for security fix."]"#;
        let c = parse_commit(raw).unwrap();
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
        let c = parse_commit(raw).unwrap();
        assert_eq!(c.r#type, "feat");
        assert_eq!(c.scope, "api");
        assert_eq!(c.message, "add endpoint");
    }

    #[test]
    fn parse_commit_extracts_flattened_array_from_prose() {
        let raw = "Here you go:\n[\"type\", \"fix\", \"scope\", \"\", \"message\", \"off-by-one\"]";
        let c = parse_commit(raw).unwrap();
        assert_eq!(c.r#type, "fix");
        assert_eq!(c.scope, "");
        assert_eq!(c.message, "off-by-one");
    }

    #[test]
    fn parse_commit_reads_three_element_array_positionally() {
        // serde deserializes a struct from a 3-element sequence positionally
        // (type, scope, message), which is a reasonable interpretation.
        let c = parse_commit(r#"["fix", "api", "off-by-one"]"#).unwrap();
        assert_eq!(c.r#type, "fix");
        assert_eq!(c.scope, "api");
        assert_eq!(c.message, "off-by-one");
    }

    #[test]
    fn parse_commit_rejects_array_without_commit_keys() {
        assert!(parse_commit(r#"["a", "b", "c", "d"]"#).is_err());
    }

    #[test]
    fn parse_commit_handles_odd_length_flattened_array() {
        // A flattened array with a dangling trailing key (no value). Pair what
        // we can and drop the orphan rather than failing.
        let raw = r#"["_type", "chore", "_scope", "deps", "message"]"#;
        let c = parse_commit(raw).unwrap();
        assert_eq!(c.r#type, "chore");
        assert_eq!(c.scope, "deps");
        assert_eq!(c.message, "");
    }

    #[test]
    fn parse_commit_reads_first_of_array_of_objects() {
        // Some models emit one commit object per changed file. Take the first.
        let raw = r#"[{"type": "docs", "scope": ".github/workflows/api-deploy-reusable.yml", "message": "Remove Stripe price ID config from API deploy workflow"}, {"type": "docs", "scope": ".github/workflows/infra-deploy-reusable.yml", "message": "Remove Stripe price ID handling from infra deploy workflow"}]"#;
        let c = parse_commit(raw).unwrap();
        assert_eq!(c.r#type, "docs");
        assert_eq!(c.scope, ".github/workflows/api-deploy-reusable.yml");
        assert_eq!(
            c.message,
            "Remove Stripe price ID config from API deploy workflow"
        );
    }
}
