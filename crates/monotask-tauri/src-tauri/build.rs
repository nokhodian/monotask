fn main() {
    tauri_build::build();

    // Embed a unix timestamp at compile time so the running app can display
    // a human-readable build version that increments on every `cargo build`.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    println!("cargo:rustc-env=BUILD_TS={ts}");
    // Rerun whenever the main source changes (ensures fresh timestamp on rebuild)
    println!("cargo:rerun-if-changed=src/main.rs");
}
