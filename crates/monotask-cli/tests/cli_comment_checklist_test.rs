use std::process::Command;

fn cli(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_app-cli"))
        .args(["--data-dir", dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("failed to run app-cli")
}

/// Helper: create board+column+card, return (board_id, col_id, card_id)
fn setup_card(tmp: &std::path::Path) -> (String, String, String) {
    let board_out = cli(tmp, &["board", "create", "TestBoard", "--json"]);
    let board: serde_json::Value = serde_json::from_slice(&board_out.stdout).unwrap();
    let board_id = board["id"].as_str().unwrap().to_string();

    let col_out = cli(tmp, &["column", "create", &board_id, "To Do", "--json"]);
    let col: serde_json::Value = serde_json::from_slice(&col_out.stdout).unwrap();
    let col_id = col["id"].as_str().unwrap().to_string();

    let card_out = cli(tmp, &["card", "create", &board_id, &col_id, "My Task", "--json"]);
    let card: serde_json::Value = serde_json::from_slice(&card_out.stdout).unwrap();
    let card_id = card["id"].as_str().unwrap().to_string();

    (board_id, col_id, card_id)
}

#[test]
fn comment_add_list_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let (board_id, _col_id, card_id) = setup_card(tmp.path());

    // Add comment
    let add_out = cli(tmp.path(), &[
        "card", "comment", "add", &board_id, &card_id, "Hello world", "--json"
    ]);
    assert!(add_out.status.success(), "stderr: {}", String::from_utf8_lossy(&add_out.stderr));
    let comment: serde_json::Value = serde_json::from_slice(&add_out.stdout).unwrap();
    let comment_id = comment["id"].as_str().unwrap().to_string();
    assert_eq!(comment["text"], "Hello world");

    // List comments
    let list_out = cli(tmp.path(), &[
        "card", "comment", "list", &board_id, &card_id, "--json"
    ]);
    assert!(list_out.status.success());
    let comments: Vec<serde_json::Value> = serde_json::from_slice(&list_out.stdout).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["text"], "Hello world");

    // Delete comment
    let del_out = cli(tmp.path(), &[
        "card", "comment", "delete", &board_id, &card_id, &comment_id, "--json"
    ]);
    assert!(del_out.status.success());

    // List again — should be empty
    let list2_out = cli(tmp.path(), &[
        "card", "comment", "list", &board_id, &card_id, "--json"
    ]);
    let comments2: Vec<serde_json::Value> = serde_json::from_slice(&list2_out.stdout).unwrap();
    assert_eq!(comments2.len(), 0);
}

#[test]
fn checklist_add_item_check() {
    let tmp = tempfile::tempdir().unwrap();
    let (board_id, _col_id, card_id) = setup_card(tmp.path());

    // Add checklist
    let cl_out = cli(tmp.path(), &[
        "checklist", "add", &board_id, &card_id, "QA Steps", "--json"
    ]);
    assert!(cl_out.status.success(), "stderr: {}", String::from_utf8_lossy(&cl_out.stderr));
    let cl: serde_json::Value = serde_json::from_slice(&cl_out.stdout).unwrap();
    let cl_id = cl["id"].as_str().unwrap().to_string();
    assert_eq!(cl["title"], "QA Steps");

    // Add item
    let item_out = cli(tmp.path(), &[
        "checklist", "item-add", &board_id, &card_id, &cl_id, "Write tests", "--json"
    ]);
    assert!(item_out.status.success(), "stderr: {}", String::from_utf8_lossy(&item_out.stderr));
    let item: serde_json::Value = serde_json::from_slice(&item_out.stdout).unwrap();
    let item_id = item["id"].as_str().unwrap().to_string();
    assert_eq!(item["checked"], false);

    // Check the item
    let check_out = cli(tmp.path(), &[
        "checklist", "item-check", &board_id, &card_id, &cl_id, &item_id, "--json"
    ]);
    assert!(check_out.status.success());
    let check_result: serde_json::Value = serde_json::from_slice(&check_out.stdout).unwrap();
    assert_eq!(check_result["checked"], true);
}
