//! `{{result:tool}}` pipes: `{{result:NAME}}` (or `{{result:NAME.field}}`) in a tool argument —
//! alone or inside other text — is replaced by the newest earlier result of the tool whose name is
//! NAME (or ends with it), unclipped. Lets a small local model hand a long transcript from one
//! tool to the next without generating it token by token.

use serde_json::Value;

/// Replaces every placeholder in `args`; errors name the placeholder that failed.
pub fn substitute(args: &Value, results: &[(String, String)]) -> Result<Value, String> {
    match args {
        Value::String(s) => expand(s, results).map(Value::String),
        Value::Array(items) => items.iter().map(|v| substitute(v, results)).collect::<Result<Vec<_>, _>>().map(Value::Array),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), substitute(v, results)?);
            }
            Ok(Value::Object(out))
        }
        _ => Ok(args.clone()),
    }
}

/// Largest text one placeholder may bring in; beyond it the call fails instead of flooding a tool.
const MAX_PIPED_CHARS: usize = 400_000;

/// Replaces every `{{result:…}}` in `s`, whether it is the whole string or sits inside other text.
fn expand(s: &str, results: &[(String, String)]) -> Result<String, String> {
    const OPEN: &str = "{{result:";
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find(OPEN) {
        let Some(len) = rest[start..].find("}}") else { break };
        out.push_str(&rest[..start]);
        let spec = rest[start + OPEN.len()..start + len].trim();
        let text = lookup(spec, results)?;
        if text.chars().count() > MAX_PIPED_CHARS {
            return Err(format!("{{{{result:{spec}}}}}: {} characters is more than the {MAX_PIPED_CHARS} a pipe may carry — use a smaller piece", text.chars().count()));
        }
        out.push_str(&text);
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn lookup(spec: &str, results: &[(String, String)]) -> Result<String, String> {
    let (name, field) = match spec.split_once('.') {
        Some((n, f)) => (n, Some(f)),
        None => (spec, None),
    };
    let (_, text) = results
        .iter()
        .rev()
        .find(|(tool, _)| tool == name || tool.ends_with(&format!("__{name}")))
        .ok_or_else(|| format!("{{{{result:{spec}}}}}: no earlier result from a tool named '{name}' in this turn — call it first"))?;
    let Some(field) = field else { return Ok(text.clone()) };
    let json: Value = serde_json::from_str(text).map_err(|_| format!("{{{{result:{spec}}}}}: that result isn't JSON, so it has no field '{field}'"))?;
    match json.get(field) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Null) | None => Err(format!("{{{{result:{spec}}}}}: the result has no field '{field}'")),
        Some(other) => Ok(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn results() -> Vec<(String, String)> {
        vec![
            ("mcp__youtube-transcript__get_transcript".into(), json!({"title": "T", "transcript": "line one\nline two"}).to_string()),
            ("mcp__other__thing".into(), "plain text".into()),
        ]
    }

    #[test]
    fn substitutes_a_json_field_by_tool_suffix() {
        let out = substitute(&json!({"pattern_name": "youtube_summary", "input_text": "{{result:get_transcript.transcript}}"}), &results()).unwrap();
        assert_eq!(out["input_text"], "line one\nline two");
        assert_eq!(out["pattern_name"], "youtube_summary");
    }

    #[test]
    fn substitutes_whole_text_and_nested_values() {
        let out = substitute(&json!({"config": {"x": ["{{result:thing}}"]}}), &results()).unwrap();
        assert_eq!(out["config"]["x"][0], "plain text");
    }

    #[test]
    fn newest_result_wins_and_other_strings_are_untouched() {
        let mut r = results();
        r.push(("mcp__other__thing".into(), "newer".into()));
        assert_eq!(substitute(&json!("{{result:thing}}"), &r).unwrap(), "newer");
        assert_eq!(substitute(&json!("say {{result:thing}} inline"), &r).unwrap(), "say newer inline");
        assert_eq!(substitute(&json!("no placeholder {{ here"), &r).unwrap(), "no placeholder {{ here");
    }

    #[test]
    fn inline_placeholders_and_the_size_cap() {
        let out = substitute(&json!("Summarize:\n{{result:get_transcript.transcript}}\n(end) {{result:thing}}"), &results()).unwrap();
        assert_eq!(out, "Summarize:\nline one\nline two\n(end) plain text");
        let big = vec![("mcp__a__big".to_string(), "x".repeat(MAX_PIPED_CHARS + 1))];
        assert!(substitute(&json!("{{result:big}}"), &big).unwrap_err().contains("more than"));
    }

    #[test]
    fn unknown_tool_or_field_is_a_clear_error() {
        assert!(substitute(&json!("{{result:nope}}"), &results()).unwrap_err().contains("no earlier result"));
        assert!(substitute(&json!("{{result:get_transcript.missing}}"), &results()).unwrap_err().contains("no field"));
        assert!(substitute(&json!("{{result:thing.x}}"), &results()).unwrap_err().contains("isn't JSON"));
    }
}
