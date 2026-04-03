//! Integration tests for board and column CLI commands.
//!
//! Covers:
//! - board create / list
//! - column create / list
//! - multiple boards with isolated column lists

use std::process::Command;

fn cli(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_app-cli"))
        .args(["--data-dir", dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("failed to run app-cli")
}

fn json(out: &std::process::Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "CLI failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    serde_json::from_slice(&out.stdout).expect("invalid JSON from CLI")
}

// ---------------------------------------------------------------------------
// board create
// ---------------------------------------------------------------------------

#[test]
fn board_create_returns_id_and_title() {
    let tmp = tempfile::tempdir().unwrap();

    let out = cli(tmp.path(), &["board", "create", "My Project", "--json"]);
    let board = json(&out);

    let id = board["id"].as_str().expect("board.id should be a string");
    assert!(!id.is_empty(), "board id should not be empty");
    assert_eq!(board["title"], "My Project");
}

#[test]
fn board_create_multiple_boards_have_unique_ids() {
    let tmp = tempfile::tempdir().unwrap();

    let b1 = json(&cli(tmp.path(), &["board", "create", "Alpha", "--json"]));
    let b2 = json(&cli(tmp.path(), &["board", "create", "Beta", "--json"]));

    let id1 = b1["id"].as_str().unwrap();
    let id2 = b2["id"].as_str().unwrap();

    assert_ne!(id1, id2, "two boards must have distinct ids");
    assert_eq!(b1["title"], "Alpha");
    assert_eq!(b2["title"], "Beta");
}

// ---------------------------------------------------------------------------
// board list
//
// board list --json returns an array of id strings, e.g. ["abc123", ...]
// ---------------------------------------------------------------------------

#[test]
fn board_list_empty_returns_empty_array() {
    let tmp = tempfile::tempdir().unwrap();

    let out = cli(tmp.path(), &["board", "list", "--json"]);
    let list = json(&out);

    assert!(list.is_array(), "board list should return a JSON array");
    assert_eq!(list.as_array().unwrap().len(), 0);
}

#[test]
fn board_list_reflects_created_boards() {
    let tmp = tempfile::tempdir().unwrap();

    let b1 = json(&cli(tmp.path(), &["board", "create", "First", "--json"]));
    let b2 = json(&cli(tmp.path(), &["board", "create", "Second", "--json"]));
    let b3 = json(&cli(tmp.path(), &["board", "create", "Third", "--json"]));

    let list = json(&cli(tmp.path(), &["board", "list", "--json"]));
    let arr = list.as_array().expect("should be an array");

    assert_eq!(arr.len(), 3, "should list exactly 3 boards");

    // board list returns id strings
    let ids: Vec<&str> = arr.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(ids.contains(&b1["id"].as_str().unwrap()));
    assert!(ids.contains(&b2["id"].as_str().unwrap()));
    assert!(ids.contains(&b3["id"].as_str().unwrap()));
}

#[test]
fn board_list_contains_id_strings() {
    let tmp = tempfile::tempdir().unwrap();

    let created = json(&cli(tmp.path(), &["board", "create", "FieldCheck", "--json"]));
    let created_id = created["id"].as_str().unwrap();

    let list = json(&cli(tmp.path(), &["board", "list", "--json"]));
    let arr = list.as_array().unwrap();

    // each element is a non-empty string (the board id)
    let entry = arr[0].as_str().expect("board list entry should be a string");
    assert!(!entry.is_empty());
    assert_eq!(entry, created_id);
}

// ---------------------------------------------------------------------------
// column create
//
// column create --json returns {"board_id": "...", "id": "..."}
// The title is visible via column list, not in the create response.
// ---------------------------------------------------------------------------

#[test]
fn column_create_returns_id_and_board_id() {
    let tmp = tempfile::tempdir().unwrap();

    let board = json(&cli(tmp.path(), &["board", "create", "ColBoard", "--json"]));
    let board_id = board["id"].as_str().unwrap();

    let col = json(&cli(
        tmp.path(),
        &["column", "create", board_id, "Backlog", "--json"],
    ));

    let col_id = col["id"].as_str().expect("column.id should be a string");
    assert!(!col_id.is_empty(), "column id should not be empty");

    // The response also carries back the board_id it was created on
    assert_eq!(
        col["board_id"].as_str().unwrap(),
        board_id,
        "column create should echo the board_id"
    );

    // The title is surfaced through column list
    let list = json(&cli(tmp.path(), &["column", "list", board_id, "--json"]));
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["title"], "Backlog");
}

#[test]
fn column_create_multiple_columns_have_unique_ids() {
    let tmp = tempfile::tempdir().unwrap();

    let board = json(&cli(tmp.path(), &["board", "create", "MultiCol", "--json"]));
    let board_id = board["id"].as_str().unwrap();

    let c1 = json(&cli(
        tmp.path(),
        &["column", "create", board_id, "Todo", "--json"],
    ));
    let c2 = json(&cli(
        tmp.path(),
        &["column", "create", board_id, "In Progress", "--json"],
    ));
    let c3 = json(&cli(
        tmp.path(),
        &["column", "create", board_id, "Done", "--json"],
    ));

    let ids: Vec<&str> = vec![
        c1["id"].as_str().unwrap(),
        c2["id"].as_str().unwrap(),
        c3["id"].as_str().unwrap(),
    ];

    // all ids must be distinct
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[1], ids[2]);
    assert_ne!(ids[0], ids[2]);
}

// ---------------------------------------------------------------------------
// column list
// ---------------------------------------------------------------------------

#[test]
fn column_list_empty_returns_empty_array() {
    let tmp = tempfile::tempdir().unwrap();

    let board = json(&cli(tmp.path(), &["board", "create", "EmptyColBoard", "--json"]));
    let board_id = board["id"].as_str().unwrap();

    let list = json(&cli(tmp.path(), &["column", "list", board_id, "--json"]));
    assert!(list.is_array(), "column list should return a JSON array");
    assert_eq!(list.as_array().unwrap().len(), 0);
}

#[test]
fn column_list_reflects_created_columns() {
    let tmp = tempfile::tempdir().unwrap();

    let board = json(&cli(tmp.path(), &["board", "create", "ListColBoard", "--json"]));
    let board_id = board["id"].as_str().unwrap();

    cli(tmp.path(), &["column", "create", board_id, "Todo", "--json"]);
    cli(tmp.path(), &["column", "create", board_id, "Doing", "--json"]);
    cli(tmp.path(), &["column", "create", board_id, "Done", "--json"]);

    let list = json(&cli(tmp.path(), &["column", "list", board_id, "--json"]));
    let arr = list.as_array().expect("should be an array");

    assert_eq!(arr.len(), 3, "should list exactly 3 columns");

    let titles: Vec<&str> = arr
        .iter()
        .map(|c| c["title"].as_str().unwrap())
        .collect();

    assert!(titles.contains(&"Todo"));
    assert!(titles.contains(&"Doing"));
    assert!(titles.contains(&"Done"));
}

#[test]
fn column_list_contains_id_and_title_fields() {
    let tmp = tempfile::tempdir().unwrap();

    let board = json(&cli(tmp.path(), &["board", "create", "FieldColBoard", "--json"]));
    let board_id = board["id"].as_str().unwrap();

    cli(
        tmp.path(),
        &["column", "create", board_id, "Sprint 1", "--json"],
    );

    let list = json(&cli(tmp.path(), &["column", "list", board_id, "--json"]));
    let col = &list.as_array().unwrap()[0];

    assert!(
        col["id"].as_str().is_some(),
        "each column entry should have an id string"
    );
    assert!(
        col["title"].as_str().is_some(),
        "each column entry should have a title string"
    );
}

// ---------------------------------------------------------------------------
// isolation: columns belong only to their board
// ---------------------------------------------------------------------------

#[test]
fn column_list_is_isolated_per_board() {
    let tmp = tempfile::tempdir().unwrap();

    let b1 = json(&cli(tmp.path(), &["board", "create", "BoardA", "--json"]));
    let b1_id = b1["id"].as_str().unwrap();

    let b2 = json(&cli(tmp.path(), &["board", "create", "BoardB", "--json"]));
    let b2_id = b2["id"].as_str().unwrap();

    // Add 2 columns to board A, 1 column to board B
    cli(tmp.path(), &["column", "create", b1_id, "A-Todo", "--json"]);
    cli(tmp.path(), &["column", "create", b1_id, "A-Done", "--json"]);
    cli(tmp.path(), &["column", "create", b2_id, "B-Backlog", "--json"]);

    let cols_a = json(&cli(tmp.path(), &["column", "list", b1_id, "--json"]));
    let cols_b = json(&cli(tmp.path(), &["column", "list", b2_id, "--json"]));

    assert_eq!(
        cols_a.as_array().unwrap().len(),
        2,
        "BoardA should have 2 columns"
    );
    assert_eq!(
        cols_b.as_array().unwrap().len(),
        1,
        "BoardB should have 1 column"
    );

    let titles_a: Vec<&str> = cols_a
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["title"].as_str().unwrap())
        .collect();
    let titles_b: Vec<&str> = cols_b
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["title"].as_str().unwrap())
        .collect();

    assert!(titles_a.contains(&"A-Todo"));
    assert!(titles_a.contains(&"A-Done"));
    assert!(!titles_a.contains(&"B-Backlog"), "B-Backlog must not bleed into BoardA");

    assert!(titles_b.contains(&"B-Backlog"));
    assert!(!titles_b.contains(&"A-Todo"), "A-Todo must not bleed into BoardB");
    assert!(!titles_b.contains(&"A-Done"), "A-Done must not bleed into BoardB");
}

// ---------------------------------------------------------------------------
// round-trip: board list preserves the id returned by board create
//
// board list returns an array of id strings; the created id must appear in it.
// ---------------------------------------------------------------------------

#[test]
fn board_create_id_matches_board_list_id() {
    let tmp = tempfile::tempdir().unwrap();

    let created = json(&cli(tmp.path(), &["board", "create", "RoundTrip", "--json"]));
    let created_id = created["id"].as_str().unwrap();

    let list = json(&cli(tmp.path(), &["board", "list", "--json"]));
    let arr = list.as_array().unwrap();

    // board list entries are plain id strings
    let found = arr.iter().any(|v| v.as_str() == Some(created_id));
    assert!(found, "board list should include the id returned by board create");
}

// ---------------------------------------------------------------------------
// round-trip: column list preserves the id returned by column create
// ---------------------------------------------------------------------------

#[test]
fn column_create_id_matches_column_list_id() {
    let tmp = tempfile::tempdir().unwrap();

    let board = json(&cli(tmp.path(), &["board", "create", "ColRoundTrip", "--json"]));
    let board_id = board["id"].as_str().unwrap();

    let created = json(&cli(
        tmp.path(),
        &["column", "create", board_id, "Sprint", "--json"],
    ));
    let created_id = created["id"].as_str().unwrap();

    let list = json(&cli(tmp.path(), &["column", "list", board_id, "--json"]));
    let arr = list.as_array().unwrap();

    let found = arr
        .iter()
        .find(|c| c["id"].as_str() == Some(created_id));

    assert!(
        found.is_some(),
        "column list should include the id returned by column create"
    );
    assert_eq!(found.unwrap()["title"], "Sprint");
}
