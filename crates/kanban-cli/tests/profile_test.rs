//! Integration tests for `profile` commands.
//!
//! Covers:
//!   - `profile show`        → exits 0, prints Pubkey / Display name / Avatar / SSH key path
//!   - `profile set-name`    → exits 0, prints confirmation
//!   - `profile show` again  → reflects the new display name
//!   - idempotent set-name   → overwriting a name works
//!   - pubkey is 64-char hex

use std::process::Command;

fn cli(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_app-cli"))
        .args(["--data-dir", dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("failed to run app-cli")
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

// ── profile show ──────────────────────────────────────────────────────────────

#[test]
fn profile_show_succeeds_and_has_expected_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cli(tmp.path(), &["profile", "show"]);

    assert!(
        out.status.success(),
        "profile show failed.\nstderr: {}",
        stderr(&out)
    );

    let text = stdout(&out);
    assert!(text.contains("Pubkey:"), "missing 'Pubkey:' in output:\n{}", text);
    assert!(text.contains("Display name:"), "missing 'Display name:' in output:\n{}", text);
    assert!(text.contains("Avatar:"), "missing 'Avatar:' in output:\n{}", text);
    assert!(text.contains("SSH key path:"), "missing 'SSH key path:' in output:\n{}", text);
}

#[test]
fn profile_show_pubkey_is_64_char_hex() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cli(tmp.path(), &["profile", "show"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let text = stdout(&out);

    // Extract the pubkey value from the "Pubkey:       <value>" line
    let pubkey_line = text
        .lines()
        .find(|l| l.starts_with("Pubkey:"))
        .expect("no 'Pubkey:' line in output");

    let pubkey = pubkey_line
        .splitn(2, ':')
        .nth(1)
        .expect("malformed Pubkey line")
        .trim();

    assert_eq!(pubkey.len(), 64, "pubkey should be 64 hex chars, got: {:?}", pubkey);
    assert!(
        pubkey.chars().all(|c| c.is_ascii_hexdigit()),
        "pubkey should be hex, got: {:?}",
        pubkey
    );
}

#[test]
fn profile_show_default_display_name_is_not_set() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cli(tmp.path(), &["profile", "show"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let text = stdout(&out);
    // Fresh identity → display name should show the default placeholder
    assert!(
        text.contains("(not set)"),
        "expected '(not set)' for display name on fresh identity, got:\n{}",
        text
    );
}

#[test]
fn profile_show_default_avatar_is_not_set() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cli(tmp.path(), &["profile", "show"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let text = stdout(&out);
    assert!(
        text.contains("not set"),
        "expected 'not set' for avatar on fresh identity, got:\n{}",
        text
    );
}

// ── profile set-name ──────────────────────────────────────────────────────────

#[test]
fn profile_set_name_succeeds_and_confirms() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cli(tmp.path(), &["profile", "set-name", "Alice"]);

    assert!(
        out.status.success(),
        "profile set-name failed.\nstderr: {}",
        stderr(&out)
    );

    let text = stdout(&out);
    assert!(
        text.contains("Alice"),
        "confirmation message should contain the new name, got:\n{}",
        text
    );
}

// ── profile show after set-name ───────────────────────────────────────────────

#[test]
fn profile_show_reflects_updated_display_name() {
    let tmp = tempfile::tempdir().unwrap();

    // Set the name
    let set_out = cli(tmp.path(), &["profile", "set-name", "Bob"]);
    assert!(
        set_out.status.success(),
        "set-name failed.\nstderr: {}",
        stderr(&set_out)
    );

    // Show should now contain the name
    let show_out = cli(tmp.path(), &["profile", "show"]);
    assert!(
        show_out.status.success(),
        "profile show failed after set-name.\nstderr: {}",
        stderr(&show_out)
    );

    let text = stdout(&show_out);
    assert!(
        text.contains("Bob"),
        "profile show should contain 'Bob' after set-name, got:\n{}",
        text
    );
    assert!(
        !text.contains("(not set)"),
        "profile show should NOT contain '(not set)' after name was set, got:\n{}",
        text
    );
}

#[test]
fn profile_show_pubkey_stable_across_calls() {
    let tmp = tempfile::tempdir().unwrap();

    let extract_pubkey = |out: &std::process::Output| -> String {
        let text = stdout(out);
        let line = text.lines().find(|l| l.starts_with("Pubkey:")).unwrap();
        line.splitn(2, ':').nth(1).unwrap().trim().to_string()
    };

    let out1 = cli(tmp.path(), &["profile", "show"]);
    assert!(out1.status.success(), "stderr: {}", stderr(&out1));
    let pk1 = extract_pubkey(&out1);

    let out2 = cli(tmp.path(), &["profile", "show"]);
    assert!(out2.status.success(), "stderr: {}", stderr(&out2));
    let pk2 = extract_pubkey(&out2);

    assert_eq!(pk1, pk2, "pubkey should be stable across multiple `profile show` calls");
}

// ── idempotent / overwrite set-name ──────────────────────────────────────────

#[test]
fn profile_set_name_can_be_overwritten() {
    let tmp = tempfile::tempdir().unwrap();

    let out1 = cli(tmp.path(), &["profile", "set-name", "Charlie"]);
    assert!(out1.status.success(), "first set-name failed: {}", stderr(&out1));

    let out2 = cli(tmp.path(), &["profile", "set-name", "Dave"]);
    assert!(out2.status.success(), "second set-name failed: {}", stderr(&out2));

    let show = cli(tmp.path(), &["profile", "show"]);
    assert!(show.status.success(), "profile show failed: {}", stderr(&show));

    let text = stdout(&show);
    assert!(
        text.contains("Dave"),
        "expected 'Dave' after overwrite, got:\n{}",
        text
    );
    assert!(
        !text.contains("Charlie"),
        "old name 'Charlie' should no longer appear, got:\n{}",
        text
    );
}

#[test]
fn profile_set_name_with_spaces_works() {
    let tmp = tempfile::tempdir().unwrap();

    let out = cli(tmp.path(), &["profile", "set-name", "Jane Doe"]);
    assert!(out.status.success(), "set-name with spaces failed: {}", stderr(&out));

    let show = cli(tmp.path(), &["profile", "show"]);
    assert!(show.status.success(), "profile show failed: {}", stderr(&show));

    let text = stdout(&show);
    assert!(
        text.contains("Jane Doe"),
        "expected 'Jane Doe' in profile show, got:\n{}",
        text
    );
}

#[test]
fn profile_set_name_preserves_pubkey() {
    let tmp = tempfile::tempdir().unwrap();

    // Grab the pubkey before set-name
    let show_before = cli(tmp.path(), &["profile", "show"]);
    assert!(show_before.status.success(), "stderr: {}", stderr(&show_before));
    let before_text = stdout(&show_before);
    let pubkey_before = before_text
        .lines()
        .find(|l| l.starts_with("Pubkey:"))
        .unwrap()
        .splitn(2, ':')
        .nth(1)
        .unwrap()
        .trim()
        .to_string();

    // Set name
    let _ = cli(tmp.path(), &["profile", "set-name", "Eve"]);

    // Pubkey should not have changed
    let show_after = cli(tmp.path(), &["profile", "show"]);
    assert!(show_after.status.success(), "stderr: {}", stderr(&show_after));
    let after_text = stdout(&show_after);
    let pubkey_after = after_text
        .lines()
        .find(|l| l.starts_with("Pubkey:"))
        .unwrap()
        .splitn(2, ':')
        .nth(1)
        .unwrap()
        .trim()
        .to_string();

    assert_eq!(
        pubkey_before, pubkey_after,
        "set-name should not change the pubkey"
    );
}
