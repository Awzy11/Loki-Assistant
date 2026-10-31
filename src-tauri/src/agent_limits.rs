//! Runtime-adjustable tool-round limits (`/steps`), kept in `~/.local/share/loki/agent_limits.json` so a
//! restart or a redeploy doesn't reset them. The config values are only the starting point.

use std::sync::atomic::{AtomicUsize, Ordering};

pub const MIN: usize = 3;
pub const MAX: usize = 200;

static ORDINARY: AtomicUsize = AtomicUsize::new(0);
static ROUTINE: AtomicUsize = AtomicUsize::new(0);

fn path() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(std::env::var_os("HOME")?).join(".local/share/loki/agent_limits.json"))
}

/// Seed from config, then let the saved file win.
pub fn init(ordinary: usize, routine: usize) {
    let (mut o, mut r) = (ordinary, routine);
    if let Some(v) = path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) {
        o = v["ordinary"].as_u64().map_or(o, |n| n as usize);
        r = v["routine"].as_u64().map_or(r, |n| n as usize);
    }
    ORDINARY.store(o.clamp(MIN, MAX), Ordering::Relaxed);
    ROUTINE.store(r.clamp(MIN, MAX), Ordering::Relaxed);
}

pub fn ordinary() -> usize {
    ORDINARY.load(Ordering::Relaxed).max(MIN)
}
pub fn routine() -> usize {
    ROUTINE.load(Ordering::Relaxed).max(MIN)
}

fn save() {
    if let Some(p) = path() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, serde_json::json!({ "ordinary": ordinary(), "routine": routine() }).to_string());
    }
}

pub fn set(routine_limit: bool, n: usize) {
    (if routine_limit { &ROUTINE } else { &ORDINARY }).store(n.clamp(MIN, MAX), Ordering::Relaxed);
    save();
}

pub fn render() -> String {
    format!("Tool steps per turn: {} for ordinary turns, {} for named routines (/bod, /eod, …).\n`/steps 40` sets ordinary, `/steps routine 100` sets routines, `/steps reset` restores the defaults. Range {MIN}–{MAX}; saved across restarts.", ordinary(), routine())
}

/// Handle `/steps …`; `None` when the message isn't that command.
pub fn command(message: &str, default_ordinary: usize, default_routine: usize) -> Option<String> {
    let mut w = message.split_whitespace();
    if !w.next()?.eq_ignore_ascii_case("/steps") {
        return None;
    }
    let args: Vec<&str> = w.collect();
    let bad = || Some(format!("Couldn't read that. {}", render()));
    match args.as_slice() {
        [] => Some(render()),
        ["reset"] => {
            set(false, default_ordinary);
            set(true, default_routine);
            Some(format!("Back to the defaults. {}", render()))
        }
        [n] => match n.parse::<usize>() {
            Ok(n) => {
                set(false, n);
                Some(format!("Ordinary turns now get {} steps. {}", ordinary(), render()))
            }
            Err(_) => bad(),
        },
        ["routine" | "routines", n] => match n.parse::<usize>() {
            Ok(n) => {
                set(true, n);
                Some(format!("Routines now get {} steps. {}", routine(), render()))
            }
            Err(_) => bad(),
        },
        _ => bad(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_and_parses() {
        std::env::set_var("HOME", std::env::temp_dir().join("loki-steps-test"));
        init(8, 20);
        assert!(command("hello", 8, 20).is_none());
        assert!(command("/steps 9999", 8, 20).unwrap().contains(&MAX.to_string()));
        assert_eq!(ordinary(), MAX);
        command("/steps routine 1", 8, 20);
        assert_eq!(routine(), MIN);
        command("/steps reset", 25, 60);
        assert_eq!((ordinary(), routine()), (25, 60));
        assert!(command("/steps nonsense", 8, 20).unwrap().starts_with("Couldn't"));
    }
}
