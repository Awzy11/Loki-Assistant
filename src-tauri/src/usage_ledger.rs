//! A local record of what the cloud engines cost and how much work was kept away from them.
//!
//! Two kinds of line go into `usage.jsonl` (in Loki's data folder): `call` lines with the tokens one Claude call used, and `offload`
//! lines noting that a local model read, fetched or condensed something so the cloud model never had to see it. `summary` adds
//! them up per day. Nothing leaves this machine; it exists so a change to how Loki spends tokens can be judged by numbers.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    let d = crate::paths::data_dir()?.join("loki");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("usage.jsonl"))
}

fn append(line: Value) {
    let Some(p) = path() else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{line}");
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// One cloud model call: `u` is the CLI result's `usage` object, `cost_usd` its reported cost.
pub fn record_call(engine: &str, u: &Value, cost_usd: f64) {
    append(json!({
        "t": now(), "kind": "call", "engine": engine,
        "input": u["input_tokens"].as_u64().unwrap_or(0),
        "cache_write": u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        "cache_read": u["cache_read_input_tokens"].as_u64().unwrap_or(0),
        "output": u["output_tokens"].as_u64().unwrap_or(0),
        "cost_usd": cost_usd,
    }));
}

/// A local model handled `chars_in` characters of material and handed `chars_out` upward (or nothing, for a prepared packet).
pub fn record_offload(kind: &str, chars_in: usize, chars_out: usize) {
    append(json!({ "t": now(), "kind": "offload", "what": kind, "chars_in": chars_in, "chars_out": chars_out }));
}

/// Per-day totals for the last `days` days, newest last.
pub fn summary(days: u64) -> Value {
    let cutoff = now().saturating_sub(days.clamp(1, 90) * 86_400);
    let text = path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    summarize(&text, cutoff)
}

fn summarize(text: &str, cutoff: u64) -> Value {
    #[derive(Default)]
    struct Day {
        calls: u64,
        input: u64,
        cache_write: u64,
        cache_read: u64,
        output: u64,
        cost: f64,
        offloads: u64,
        chars_kept_from_cloud: u64,
    }
    let mut by_day: BTreeMap<String, Day> = BTreeMap::new();
    // One `date` lookup per hour of the log, not per line.
    let mut hour_to_day: std::collections::HashMap<u64, String> = std::collections::HashMap::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let t = v["t"].as_u64().unwrap_or(0);
        if t < cutoff {
            continue;
        }
        let day = hour_to_day.entry(t / 3600).or_insert_with(|| crate::conversation_log::date_format_at(t as i64, "%Y-%m-%d")).clone();
        let d = by_day.entry(day).or_default();
        match v["kind"].as_str() {
            Some("call") => {
                d.calls += 1;
                d.input += v["input"].as_u64().unwrap_or(0);
                d.cache_write += v["cache_write"].as_u64().unwrap_or(0);
                d.cache_read += v["cache_read"].as_u64().unwrap_or(0);
                d.output += v["output"].as_u64().unwrap_or(0);
                d.cost += v["cost_usd"].as_f64().unwrap_or(0.0);
            }
            Some("offload") => {
                d.offloads += 1;
                d.chars_kept_from_cloud += v["chars_in"].as_u64().unwrap_or(0).saturating_sub(v["chars_out"].as_u64().unwrap_or(0));
            }
            _ => {}
        }
    }
    let days: Vec<Value> = by_day
        .into_iter()
        .map(|(day, d)| {
            json!({
                "day": day, "calls": d.calls, "input": d.input, "cache_write": d.cache_write, "cache_read": d.cache_read, "output": d.output,
                "cost_usd": (d.cost * 10_000.0).round() / 10_000.0,
                "avg_context": if d.calls > 0 { (d.input + d.cache_write + d.cache_read) / d.calls } else { 0 },
                "offloads": d.offloads, "chars_kept_from_cloud": d.chars_kept_from_cloud,
            })
        })
        .collect();
    json!({ "days": days })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_calls_and_offloads_per_day() {
        let day = 1_700_000_000u64;
        let text = [
            json!({"t": day, "kind": "call", "engine": "claude-inference", "input": 10, "cache_write": 100, "cache_read": 1000, "output": 50, "cost_usd": 0.5}),
            json!({"t": day + 5, "kind": "call", "engine": "claude-inference", "input": 20, "cache_write": 0, "cache_read": 1200, "output": 70, "cost_usd": 0.25}),
            json!({"t": day + 9, "kind": "offload", "what": "fetch", "chars_in": 9000, "chars_out": 1500}),
            json!({"t": 5, "kind": "call", "input": 999}),
        ]
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        let s = summarize(&text, day - 10);
        let d = &s["days"][0];
        assert_eq!(s["days"].as_array().unwrap().len(), 1);
        assert_eq!(d["calls"], 2);
        assert_eq!(d["cache_read"], 2200);
        assert_eq!(d["avg_context"], (30 + 100 + 2200) / 2);
        assert_eq!(d["offloads"], 1);
        assert_eq!(d["chars_kept_from_cloud"], 7500);
        assert_eq!(d["cost_usd"], 0.75);
    }
}
