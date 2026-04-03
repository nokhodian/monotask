// crates/kanban-cli/tests/scaffold_smoke_test.rs
use std::process::Command;

fn cli(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_app-cli"))
        .args(["--data-dir", dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("failed to run app-cli")
}

#[test]
fn board_column_card_create_list() {
    let tmp = tempfile::tempdir().unwrap();

    // Create board
    let board_out = cli(tmp.path(), &["board", "create", "MyBoard", "--json"]);
    assert!(board_out.status.success(), "{}", String::from_utf8_lossy(&board_out.stderr));
    let board: serde_json::Value = serde_json::from_slice(&board_out.stdout).unwrap();
    let board_id = board["id"].as_str().unwrap();

    // Create column
    let col_out = cli(tmp.path(), &["column", "create", board_id, "To Do", "--json"]);
    assert!(col_out.status.success(), "{}", String::from_utf8_lossy(&col_out.stderr));
    let col: serde_json::Value = serde_json::from_slice(&col_out.stdout).unwrap();
    let col_id = col["id"].as_str().unwrap();

    // Create card
    let card_out = cli(tmp.path(), &["card", "create", board_id, col_id, "Deploy API", "--json"]);
    assert!(card_out.status.success(), "{}", String::from_utf8_lossy(&card_out.stderr));
    let card: serde_json::Value = serde_json::from_slice(&card_out.stdout).unwrap();
    assert_eq!(card["title"], "Deploy API");

    // List boards
    let list_out = cli(tmp.path(), &["board", "list", "--json"]);
    assert!(list_out.status.success(), "{}", String::from_utf8_lossy(&list_out.stderr));
    let boards: Vec<String> = serde_json::from_slice(&list_out.stdout).unwrap();
    assert_eq!(boards.len(), 1);
}
