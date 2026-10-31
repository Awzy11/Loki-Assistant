//! `loki-device <tool> '<json input>'` — runs one `device_*` tool on this machine and prints the
//! result. The same policy module (`device.rs`: protected credential paths, the `[desktop]`
//! write/shell switches in `~/.config/loki/config.toml`) the desktop app's `device_call` uses, so
//! other clients that can't embed it (the QML sidebar) get identical behavior.
//!
//! Output: the tool's text on stdout. Exit code 0 on success, 1 on a tool error (message on stdout
//! too, so a caller can pass it straight back as `{content, is_error: true}`).
use loki_lib::{config, device};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(tool), Some(input)) = (args.next(), args.next()) else {
        eprintln!("usage: loki-device <device_tool> '<json input>'");
        std::process::exit(2);
    };
    let input: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => {
            println!("input is not valid JSON: {e}");
            std::process::exit(1);
        }
    };
    match device::call(&tool, &input, &config::load_desktop()) {
        Ok(text) => print!("{text}"),
        Err(e) => {
            print!("{e}");
            std::process::exit(1);
        }
    }
}
