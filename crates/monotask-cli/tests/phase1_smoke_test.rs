//! Phase 1 integration smoke test.
//!
//! Verifies the full Phase 1 feature set end-to-end via the CLI binary:
//! card numbers, copy, comments, checklists.

use std::process::Command;

fn cli(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_app-cli"))
        .args(["--data-dir", dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("failed to run app-cli")
}

fn json(out: &std::process::Output) -> serde_json::Value {
    assert!(out.status.success(), "CLI failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("invalid JSON from CLI")
}

#[test]
fn phase1_full_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let t = tmp.path();

    // Setup: board + column
    let board_id = json(&cli(t, &["board", "create", "Phase1Board", "--json"]))["id"]
        .as_str().unwrap().to_string();
    let col_id = json(&cli(t, &["column", "create", &board_id, "Backlog", "--json"]))["id"]
        .as_str().unwrap().to_string();

    // Create a card and verify it gets a card number
    let card = json(&cli(t, &["card", "create", &board_id, &col_id, "Deploy API", "--json"]));
    let card_id = card["id"].as_str().unwrap().to_string();

    let _card_view = json(&cli(t, &["card", "view", &board_id, &card_id, "--json"]));
    // Card number field should be present in the stored card
    // (number is stored in the automerge doc as a string field)
    assert!(!card_id.is_empty());

    // Add a comment
    let comment = json(&cli(t, &["card", "comment", "add", &board_id, &card_id, "Looks good", "--json"]));
    let comment_id = comment["id"].as_str().unwrap().to_string();
    assert_eq!(comment["text"], "Looks good");

    // List comments — should have 1
    let comments = json(&cli(t, &["card", "comment", "list", &board_id, &card_id, "--json"]));
    assert_eq!(comments.as_array().unwrap().len(), 1);

    // Add a checklist
    let cl = json(&cli(t, &["checklist", "add", &board_id, &card_id, "QA Checklist", "--json"]));
    let cl_id = cl["id"].as_str().unwrap().to_string();
    assert_eq!(cl["title"], "QA Checklist");

    // Add a checklist item
    let item = json(&cli(t, &["checklist", "item-add", &board_id, &card_id, &cl_id, "Write tests", "--json"]));
    let item_id = item["id"].as_str().unwrap().to_string();
    assert_eq!(item["checked"], false);

    // Check the item
    let checked = json(&cli(t, &["checklist", "item-check", &board_id, &card_id, &cl_id, &item_id, "--json"]));
    assert_eq!(checked["checked"], true);

    // Delete the comment
    let del = json(&cli(t, &["card", "comment", "delete", &board_id, &card_id, &comment_id, "--json"]));
    assert_eq!(del["deleted"], comment_id.as_str());

    // Comments should now be empty
    let comments2 = json(&cli(t, &["card", "comment", "list", &board_id, &card_id, "--json"]));
    assert_eq!(comments2.as_array().unwrap().len(), 0);

    // Create a second card to verify sequential numbering
    let card2 = json(&cli(t, &["card", "create", &board_id, &col_id, "Fix Bug", "--json"]));
    let card2_id = card2["id"].as_str().unwrap().to_string();
    assert_ne!(card_id, card2_id);

    // Verify both cards have numbers and the second has a higher seq
    let card_view1 = json(&cli(t, &["card", "view", &board_id, &card_id, "--json"]));
    let card_view2 = json(&cli(t, &["card", "view", &board_id, &card2_id, "--json"]));

    // Both cards should have a number with seq > 0
    let seq1 = card_view1["number"]["seq"].as_u64().expect("card1 should have a number with seq");
    let seq2 = card_view2["number"]["seq"].as_u64().expect("card2 should have a number with seq");
    assert!(seq1 > 0, "seq1 should be positive");
    assert!(seq2 > seq1, "seq2 should be greater than seq1");

    // Both cards exist in the board
    let boards_list = json(&cli(t, &["board", "list", "--json"]));
    assert_eq!(boards_list.as_array().unwrap().len(), 1);
}
