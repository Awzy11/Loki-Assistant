fn main() {
    // Declaring the app's own commands gives each an `allow-<name>` permission. A remote-origin
    // window (the desktop app opens the server's URL) can only call a command that a capability
    // names — without these, every `device_call` fails with "not allowed by ACL".
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&["capture_screenshot", "device_call"])))
        .expect("failed to run tauri-build");
}
