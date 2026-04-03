//! Integration tests for space commands.
//!
//! Tests cover: space create, space list, space info,
//! space invite generate, space boards add/list,
//! space members list.

use std::process::Command;

fn cli(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_app-cli"))
        .args(["--data-dir", dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("failed to run app-cli")
}

fn stdout(out: &std::process::Output) -> String {
    assert!(
        out.status.success(),
        "CLI failed (exit {:?}):\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Extract the space ID from "Created Space: <name> (<id>)"
fn extract_space_id(create_output: &str) -> String {
    let start = create_output.rfind('(').expect("no '(' in create output");
    let end = create_output.rfind(')').expect("no ')' in create output");
    create_output[start + 1..end].trim().to_string()
}

// ---------------------------------------------------------------------------
// space create
// ---------------------------------------------------------------------------

#[test]
fn space_create_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let out = stdout(&cli(tmp.path(), &["space", "create", "TestSpace"]));
    assert!(
        out.contains("Created Space:"),
        "expected 'Created Space:' in output, got: {out}"
    );
    assert!(
        out.contains("TestSpace"),
        "expected space name 'TestSpace' in output, got: {out}"
    );
}

#[test]
fn space_create_returns_id() {
    let tmp = tempfile::tempdir().unwrap();
    let out = stdout(&cli(tmp.path(), &["space", "create", "IDSpace"]));
    // The ID is a UUID wrapped in parentheses: "Created Space: IDSpace (<uuid>)"
    let space_id = extract_space_id(&out);
    assert!(
        !space_id.is_empty(),
        "extracted space_id should not be empty"
    );
    // UUID v4 format: 8-4-4-4-12 hex chars
    assert_eq!(
        space_id.len(),
        36,
        "space_id should be a UUID (36 chars), got: {space_id}"
    );
}

// ---------------------------------------------------------------------------
// space list
// ---------------------------------------------------------------------------

#[test]
fn space_list_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let out = stdout(&cli(tmp.path(), &["space", "list"]));
    assert!(
        out.contains("No spaces found"),
        "expected 'No spaces found' in empty list output, got: {out}"
    );
}

#[test]
fn space_list_shows_created_spaces() {
    let tmp = tempfile::tempdir().unwrap();
    stdout(&cli(tmp.path(), &["space", "create", "Alpha"]));
    stdout(&cli(tmp.path(), &["space", "create", "Beta"]));

    let out = stdout(&cli(tmp.path(), &["space", "list"]));
    assert!(
        out.contains("Alpha"),
        "expected 'Alpha' in space list, got: {out}"
    );
    assert!(
        out.contains("Beta"),
        "expected 'Beta' in space list, got: {out}"
    );
    // Each row is "<id> | <name> | <N> members"
    assert!(
        out.contains("members"),
        "expected 'members' column in space list, got: {out}"
    );
}

// ---------------------------------------------------------------------------
// space info
// ---------------------------------------------------------------------------

#[test]
fn space_info_shows_details() {
    let tmp = tempfile::tempdir().unwrap();
    let create_out = stdout(&cli(tmp.path(), &["space", "create", "InfoSpace"]));
    let space_id = extract_space_id(&create_out);

    let out = stdout(&cli(tmp.path(), &["space", "info", &space_id]));
    assert!(
        out.contains("InfoSpace"),
        "expected space name in info output, got: {out}"
    );
    assert!(
        out.contains(&space_id),
        "expected space_id in info output, got: {out}"
    );
    assert!(
        out.contains("Owner:"),
        "expected 'Owner:' in info output, got: {out}"
    );
    assert!(
        out.contains("Members"),
        "expected 'Members' in info output, got: {out}"
    );
    assert!(
        out.contains("Boards"),
        "expected 'Boards' in info output, got: {out}"
    );
}

#[test]
fn space_info_unknown_id_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cli(tmp.path(), &["space", "info", "00000000-0000-0000-0000-000000000000"]);
    assert!(
        !out.status.success(),
        "expected non-zero exit for unknown space id"
    );
}

// ---------------------------------------------------------------------------
// space invite generate
// ---------------------------------------------------------------------------

#[test]
fn space_invite_generate_returns_token() {
    let tmp = tempfile::tempdir().unwrap();
    let create_out = stdout(&cli(tmp.path(), &["space", "create", "InviteSpace"]));
    let space_id = extract_space_id(&create_out);

    let token_out = stdout(&cli(tmp.path(), &["space", "invite", "generate", &space_id]));
    // Token is printed as a single non-empty line (base58-encoded bytes)
    assert!(
        !token_out.is_empty(),
        "invite token should not be empty"
    );
    // Tokens should not contain spaces (they are base58 strings)
    assert!(
        !token_out.contains(' '),
        "invite token should not contain spaces, got: {token_out}"
    );
}

#[test]
fn space_invite_generate_twice_both_succeed() {
    // Each call to `invite generate` revokes previous invites and issues a new one.
    // The token embeds a seconds-granularity timestamp, so two calls within the
    // same second will produce identical tokens — that is expected behaviour.
    // This test simply verifies that both calls succeed and return a non-empty token.
    let tmp = tempfile::tempdir().unwrap();
    let create_out = stdout(&cli(tmp.path(), &["space", "create", "InviteSpace2"]));
    let space_id = extract_space_id(&create_out);

    let token1 = stdout(&cli(tmp.path(), &["space", "invite", "generate", &space_id]));
    let token2 = stdout(&cli(tmp.path(), &["space", "invite", "generate", &space_id]));
    assert!(!token1.is_empty(), "first invite token should not be empty");
    assert!(!token2.is_empty(), "second invite token should not be empty");
    // Both tokens must be non-whitespace base58 strings
    assert!(!token1.contains(' '), "token1 should have no spaces");
    assert!(!token2.contains(' '), "token2 should have no spaces");
}

// ---------------------------------------------------------------------------
// space boards add / list
// ---------------------------------------------------------------------------

#[test]
fn space_boards_add_and_list() {
    let tmp = tempfile::tempdir().unwrap();

    // Create a real board first so the board ID exists
    let board_out = String::from_utf8_lossy(
        &cli(tmp.path(), &["board", "create", "BoardForSpace", "--json"]).stdout,
    )
    .trim()
    .to_string();
    let board_val: serde_json::Value =
        serde_json::from_str(&board_out).expect("board create should return JSON");
    let board_id = board_val["id"].as_str().unwrap().to_string();

    let create_out = stdout(&cli(tmp.path(), &["space", "create", "BoardsSpace"]));
    let space_id = extract_space_id(&create_out);

    // Add the board
    let add_out = stdout(&cli(tmp.path(), &["space", "boards", "add", &space_id, &board_id]));
    assert!(
        add_out.contains("Added board"),
        "expected 'Added board' in output, got: {add_out}"
    );
    assert!(
        add_out.contains(&board_id),
        "expected board_id in add output, got: {add_out}"
    );

    // List boards in the space
    let list_out = stdout(&cli(tmp.path(), &["space", "boards", "list", &space_id]));
    assert!(
        list_out.contains(&board_id),
        "expected board_id in boards list, got: {list_out}"
    );
}

#[test]
fn space_boards_list_empty_when_no_boards_added() {
    let tmp = tempfile::tempdir().unwrap();
    let create_out = stdout(&cli(tmp.path(), &["space", "create", "EmptyBoardsSpace"]));
    let space_id = extract_space_id(&create_out);

    // boards list on a space with no boards should succeed with empty output
    let out = cli(tmp.path(), &["space", "boards", "list", &space_id]);
    assert!(
        out.status.success(),
        "boards list should succeed even when empty; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let list_out = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(list_out.is_empty(), "expected empty output for space with no boards, got: {list_out}");
}

// ---------------------------------------------------------------------------
// space members list
// ---------------------------------------------------------------------------

#[test]
fn space_members_list_includes_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let create_out = stdout(&cli(tmp.path(), &["space", "create", "MembersSpace"]));
    let space_id = extract_space_id(&create_out);

    let members_out = stdout(&cli(tmp.path(), &["space", "members", "list", &space_id]));
    // The owner is automatically added as a member; the output shows pubkey + display_name
    // We just check the list is non-empty (at least one line = the owner)
    assert!(
        !members_out.is_empty(),
        "members list should include at least the owner, got empty output"
    );
}

#[test]
fn space_members_list_no_kicked_marker_for_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let create_out = stdout(&cli(tmp.path(), &["space", "create", "MembersSpace2"]));
    let space_id = extract_space_id(&create_out);

    let members_out = stdout(&cli(tmp.path(), &["space", "members", "list", &space_id]));
    assert!(
        !members_out.contains("[kicked]"),
        "owner should not be marked as kicked, got: {members_out}"
    );
}

// ---------------------------------------------------------------------------
// space invite revoke
// ---------------------------------------------------------------------------

#[test]
fn space_invite_revoke_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let create_out = stdout(&cli(tmp.path(), &["space", "create", "RevokeSpace"]));
    let space_id = extract_space_id(&create_out);

    // Generate an invite first so there is something to revoke
    stdout(&cli(tmp.path(), &["space", "invite", "generate", &space_id]));

    let revoke_out = stdout(&cli(tmp.path(), &["space", "invite", "revoke", &space_id]));
    assert!(
        revoke_out.contains("Revoked"),
        "expected 'Revoked' in output, got: {revoke_out}"
    );
    assert!(
        revoke_out.contains(&space_id),
        "expected space_id in revoke output, got: {revoke_out}"
    );
}

// ---------------------------------------------------------------------------
// space info reflects boards added
// ---------------------------------------------------------------------------

#[test]
fn space_info_shows_added_board() {
    let tmp = tempfile::tempdir().unwrap();

    // Create a board
    let board_json = String::from_utf8_lossy(
        &cli(tmp.path(), &["board", "create", "InfoBoardSpace", "--json"]).stdout,
    )
    .trim()
    .to_string();
    let board_val: serde_json::Value =
        serde_json::from_str(&board_json).expect("board create should return JSON");
    let board_id = board_val["id"].as_str().unwrap().to_string();

    // Create space and add the board
    let create_out = stdout(&cli(tmp.path(), &["space", "create", "InfoWithBoard"]));
    let space_id = extract_space_id(&create_out);
    stdout(&cli(tmp.path(), &["space", "boards", "add", &space_id, &board_id]));

    // space info should now list the board
    let info_out = stdout(&cli(tmp.path(), &["space", "info", &space_id]));
    assert!(
        info_out.contains(&board_id),
        "space info should list the added board, got: {info_out}"
    );
    // Boards count should be 1
    assert!(
        info_out.contains("Boards (1)"),
        "expected 'Boards (1)' in info output, got: {info_out}"
    );
}
