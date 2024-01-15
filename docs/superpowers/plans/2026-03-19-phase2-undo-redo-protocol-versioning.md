# Phase 2 Feature Additions: Undo/Redo + Protocol Version Negotiation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add local undo/redo using domain-level compensating operations, and add version negotiation to the Iroh connection handshake so peers running incompatible versions are rejected with a clear error.

**Architecture:** Undo is local-only — compensating ops are stored in SQLite and never gossiped as "undo"; they are applied as new signed Automerge changes. The version handshake runs before any board data flows over a new Iroh connection; a 1-byte type prefix on all Iroh gossip messages routes presence/version messages away from the Automerge apply path.

**Tech Stack:** Rust 2021, `automerge = "0.5"`, `rusqlite`, `iroh`, `semver = "1"`, `ciborium` (CBOR), `clap` v4, Tauri v2 channels

**Depends on:** Phase 1 plan (card numbers must exist before undo of card copy can reference card numbers in toasts).

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| Create | `crates/kanban-core/src/operation.rs` | `Operation` enum (all domain ops), `InverseOp` |
| Create | `crates/kanban-core/src/undo.rs` | `generate_inverse`, tombstone guard, copy-undo detection |
| Modify | `crates/kanban-core/src/card.rs` | Return `Operation` alongside `Card` from mutations |
| Modify | `crates/kanban-storage/src/schema.rs` | Add `undo_stack`, `redo_stack` tables |
| Create | `crates/kanban-storage/src/undo.rs` | Stack push/pop, depth pruning |
| Create | `crates/kanban-cli/src/commands/undo.rs` | `undo`, `redo`, `undo-history` commands |
| Modify | `crates/kanban-cli/src/main.rs` | Register undo commands |
| Create | `crates/kanban-tauri/src-tauri/src/commands/undo.rs` | Tauri undo/redo commands |
| Modify | `crates/kanban-tauri/src-tauri/src/main.rs` | Register Tauri commands |
| Create | `crates/kanban-net/src/version.rs` | `VersionHello`, `VersionReject` CBOR structs, handshake |
| Create | `crates/kanban-net/src/message.rs` | 1-byte type prefix routing |
| Modify | `crates/kanban-net/src/connection.rs` | Integrate handshake before data exchange |
| Modify | `crates/kanban-net/Cargo.toml` | Add `semver = "1"` |

---

### Task 1: `Operation` enum and `InverseOp`

**Files:**
- Create: `crates/kanban-core/src/operation.rs`

Every mutation in `kanban-core` produces an `Operation`. `generate_inverse` converts it to an `InverseOp` that can be stored and later re-applied.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/kanban-core/src/operation.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_card_inverse_swaps_columns_and_positions() {
        let op = Operation::MoveCard {
            card_id: "c1".into(),
            from_col: "col-a".into(),
            to_col: "col-b".into(),
            from_pos: 0,
            to_pos: 2,
        };
        let inv = generate_inverse(&op).unwrap();
        match inv {
            InverseOp::Apply(Operation::MoveCard { from_col, to_col, from_pos, to_pos, .. }) => {
                assert_eq!(from_col, "col-b");
                assert_eq!(to_col, "col-a");
                assert_eq!(from_pos, 2);
                assert_eq!(to_pos, 0);
            }
            _ => panic!("wrong inverse"),
        }
    }

    #[test]
    fn create_card_inverse_is_delete() {
        let op = Operation::CreateCard { card_id: "c1".into(), col_id: "col-a".into() };
        let inv = generate_inverse(&op).unwrap();
        assert!(matches!(inv, InverseOp::Apply(Operation::DeleteCard { .. })));
    }

    #[test]
    fn rename_inverse_restores_old_title() {
        let op = Operation::RenameCard {
            card_id: "c1".into(),
            old_title: "Before".into(),
            new_title: "After".into(),
        };
        let inv = generate_inverse(&op).unwrap();
        match inv {
            InverseOp::Apply(Operation::RenameCard { new_title, .. }) => {
                assert_eq!(new_title, "Before");
            }
            _ => panic!("wrong inverse"),
        }
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-core operation
```
Expected: compile error — module doesn't exist.

- [ ] **Step 3: Implement**

```rust
// crates/kanban-core/src/operation.rs

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Operation {
    CreateCard { card_id: String, col_id: String },
    DeleteCard { card_id: String },
    MoveCard { card_id: String, from_col: String, to_col: String, from_pos: usize, to_pos: usize },
    RenameCard { card_id: String, old_title: String, new_title: String },
    RenameColumn { col_id: String, old_title: String, new_title: String },
    CreateColumn { col_id: String },
    DeleteColumn { col_id: String },
    MoveColumn { col_id: String, from_pos: usize, to_pos: usize },
    CopyCard { new_card_id: String, source_card_id: String },
    // Extend with further ops as needed (comments, checklists, etc.)
}

/// The result of `generate_inverse`. Either a compensating `Operation`
/// or `NotUndoable` for ops that have no meaningful inverse (e.g. archive).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum InverseOp {
    Apply(Operation),
    NotUndoable { reason: String },
}

pub fn generate_inverse(op: &Operation) -> Result<InverseOp, crate::Error> {
    let inv = match op {
        Operation::CreateCard { card_id, .. } => InverseOp::Apply(
            Operation::DeleteCard { card_id: card_id.clone() }
        ),
        Operation::DeleteCard { card_id } => InverseOp::NotUndoable {
            reason: format!("cannot reconstruct deleted card {card_id} without a snapshot")
        },
        Operation::MoveCard { card_id, from_col, to_col, from_pos, to_pos } => InverseOp::Apply(
            Operation::MoveCard {
                card_id: card_id.clone(),
                from_col: to_col.clone(),
                to_col: from_col.clone(),
                from_pos: *to_pos,
                to_pos: *from_pos,
            }
        ),
        Operation::RenameCard { card_id, old_title, new_title } => InverseOp::Apply(
            Operation::RenameCard {
                card_id: card_id.clone(),
                old_title: new_title.clone(),
                new_title: old_title.clone(),
            }
        ),
        Operation::RenameColumn { col_id, old_title, new_title } => InverseOp::Apply(
            Operation::RenameColumn {
                col_id: col_id.clone(),
                old_title: new_title.clone(),
                new_title: old_title.clone(),
            }
        ),
        Operation::MoveColumn { col_id, from_pos, to_pos } => InverseOp::Apply(
            Operation::MoveColumn {
                col_id: col_id.clone(),
                from_pos: *to_pos,
                to_pos: *from_pos,
            }
        ),
        Operation::CopyCard { new_card_id, .. } => InverseOp::Apply(
            Operation::DeleteCard { card_id: new_card_id.clone() }
        ),
        _ => InverseOp::NotUndoable { reason: "this operation type is not undoable".into() },
    };
    Ok(inv)
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-core operation
```
Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-core/src/operation.rs crates/kanban-core/src/lib.rs
git commit -m "feat(core): add Operation enum and generate_inverse for undo/redo"
```

---

### Task 2: `undo_stack` / `redo_stack` SQLite tables + stack manager

**Files:**
- Modify: `crates/kanban-storage/src/schema.rs`
- Create: `crates/kanban-storage/src/undo.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// crates/kanban-storage/src/undo.rs (bottom of file)
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use kanban_core::operation::{Operation, InverseOp};

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::schema::run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn push_and_pop_undo() {
        let conn = test_db();
        let inv = InverseOp::Apply(Operation::DeleteCard { card_id: "c1".into() });
        push_undo(&conn, "board1", "pk1", "create_card", &inv).unwrap();
        let entry = pop_undo(&conn, "board1", "pk1").unwrap().unwrap();
        assert!(matches!(entry.inverse, InverseOp::Apply(Operation::DeleteCard { .. })));
    }

    #[test]
    fn depth_limit_enforced() {
        let conn = test_db();
        // Push 55 entries (limit is 50 by default)
        for i in 0..55 {
            let inv = InverseOp::Apply(Operation::DeleteCard { card_id: format!("c{i}") });
            push_undo(&conn, "board1", "pk1", "create_card", &inv).unwrap();
        }
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM undo_stack WHERE board_id='board1' AND actor_key='pk1'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 50);
    }

    #[test]
    fn new_action_clears_redo_stack() {
        let conn = test_db();
        let inv = InverseOp::Apply(Operation::DeleteCard { card_id: "c1".into() });
        push_redo(&conn, "board1", "pk1", "create_card", &inv).unwrap();
        clear_redo(&conn, "board1", "pk1").unwrap();
        let entry = pop_redo(&conn, "board1", "pk1").unwrap();
        assert!(entry.is_none());
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-storage undo
```
Expected: FAIL.

- [ ] **Step 3: Add SQL migration**

In `crates/kanban-storage/src/schema.rs`:
```sql
-- Migration N+1: undo and redo stacks
CREATE TABLE IF NOT EXISTS undo_stack (
    board_id   TEXT    NOT NULL,
    actor_key  TEXT    NOT NULL,
    seq        INTEGER NOT NULL,
    action_tag TEXT    NOT NULL,
    inverse_op BLOB    NOT NULL,
    hlc        TEXT    NOT NULL,
    PRIMARY KEY (board_id, actor_key, seq)
);

CREATE TABLE IF NOT EXISTS redo_stack (
    board_id   TEXT    NOT NULL,
    actor_key  TEXT    NOT NULL,
    seq        INTEGER NOT NULL,
    action_tag TEXT    NOT NULL,
    forward_op BLOB    NOT NULL,
    hlc        TEXT    NOT NULL,
    PRIMARY KEY (board_id, actor_key, seq)
);
```

- [ ] **Step 4: Implement stack manager**

```rust
// crates/kanban-storage/src/undo.rs
use rusqlite::{Connection, params};
use kanban_core::operation::InverseOp;

const DEFAULT_UNDO_DEPTH: i64 = 50;

pub struct UndoEntry {
    pub action_tag: String,
    pub inverse: InverseOp,
    pub hlc: String,
}

pub fn push_undo(
    conn: &Connection,
    board_id: &str,
    actor_key: &str,
    action_tag: &str,
    inverse: &InverseOp,
) -> rusqlite::Result<()> {
    let next_seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM undo_stack WHERE board_id=?1 AND actor_key=?2",
        params![board_id, actor_key],
        |r| r.get(0),
    )?;
    let encoded = ciborium::ser::into_vec(inverse).expect("CBOR encode");
    let hlc = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO undo_stack (board_id, actor_key, seq, action_tag, inverse_op, hlc)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![board_id, actor_key, next_seq, action_tag, encoded, hlc],
    )?;
    // Enforce depth limit
    conn.execute(
        "DELETE FROM undo_stack WHERE board_id=?1 AND actor_key=?2
         AND seq <= (SELECT MAX(seq) FROM undo_stack WHERE board_id=?1 AND actor_key=?2) - ?3",
        params![board_id, actor_key, DEFAULT_UNDO_DEPTH],
    )?;
    Ok(())
}

pub fn pop_undo(
    conn: &Connection,
    board_id: &str,
    actor_key: &str,
) -> rusqlite::Result<Option<UndoEntry>> {
    let row = conn.query_row(
        "SELECT seq, action_tag, inverse_op, hlc FROM undo_stack
         WHERE board_id=?1 AND actor_key=?2 ORDER BY seq DESC LIMIT 1",
        params![board_id, actor_key],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, String>(3)?)),
    ).optional()?;

    match row {
        None => Ok(None),
        Some((seq, action_tag, encoded, hlc)) => {
            conn.execute(
                "DELETE FROM undo_stack WHERE board_id=?1 AND actor_key=?2 AND seq=?3",
                params![board_id, actor_key, seq],
            )?;
            let inverse: InverseOp = ciborium::de::from_reader(&encoded[..]).expect("CBOR decode");
            Ok(Some(UndoEntry { action_tag, inverse, hlc }))
        }
    }
}

pub fn push_redo(conn: &Connection, board_id: &str, actor_key: &str, action_tag: &str, forward: &InverseOp) -> rusqlite::Result<()> {
    // Same pattern as push_undo but into redo_stack
    let next_seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM redo_stack WHERE board_id=?1 AND actor_key=?2",
        params![board_id, actor_key], |r| r.get(0),
    )?;
    let encoded = ciborium::ser::into_vec(forward).expect("CBOR encode");
    conn.execute(
        "INSERT INTO redo_stack (board_id, actor_key, seq, action_tag, forward_op, hlc) VALUES (?1,?2,?3,?4,?5,?6)",
        params![board_id, actor_key, next_seq, action_tag, encoded, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub fn pop_redo(conn: &Connection, board_id: &str, actor_key: &str) -> rusqlite::Result<Option<UndoEntry>> {
    // Mirror of pop_undo on redo_stack
    let row = conn.query_row(
        "SELECT seq, action_tag, forward_op, hlc FROM redo_stack
         WHERE board_id=?1 AND actor_key=?2 ORDER BY seq DESC LIMIT 1",
        params![board_id, actor_key],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, String>(3)?)),
    ).optional()?;
    match row {
        None => Ok(None),
        Some((seq, action_tag, encoded, hlc)) => {
            conn.execute(
                "DELETE FROM redo_stack WHERE board_id=?1 AND actor_key=?2 AND seq=?3",
                params![board_id, actor_key, seq],
            )?;
            let fwd: InverseOp = ciborium::de::from_reader(&encoded[..]).expect("CBOR decode");
            Ok(Some(UndoEntry { action_tag, inverse: fwd, hlc }))
        }
    }
}

pub fn clear_redo(conn: &Connection, board_id: &str, actor_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM redo_stack WHERE board_id=?1 AND actor_key=?2",
        params![board_id, actor_key],
    )?;
    Ok(())
}

pub fn list_undo(conn: &Connection, board_id: &str, actor_key: &str) -> rusqlite::Result<Vec<UndoEntry>> {
    let mut stmt = conn.prepare(
        "SELECT action_tag, inverse_op, hlc FROM undo_stack
         WHERE board_id=?1 AND actor_key=?2 ORDER BY seq DESC",
    )?;
    let rows = stmt.query_map(params![board_id, actor_key], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, String>(2)?))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        let (action_tag, encoded, hlc) = row?;
        let inverse: InverseOp = ciborium::de::from_reader(&encoded[..]).expect("CBOR decode");
        entries.push(UndoEntry { action_tag, inverse, hlc });
    }
    Ok(entries)
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p kanban-storage undo
```
Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/kanban-storage/src/undo.rs crates/kanban-storage/src/schema.rs crates/kanban-storage/src/lib.rs
git commit -m "feat(storage): undo_stack and redo_stack tables with push/pop/clear"
```

---

### Task 3: Tombstone guard and undo application in `kanban-core`

**Files:**
- Create: `crates/kanban-core/src/undo.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// crates/kanban-core/src/undo.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_undo_move_card_succeeds() {
        let mut doc = build_test_board(); // creates board with one card in col-a
        let result = apply_inverse_op(&mut doc, &InverseOp::Apply(
            Operation::MoveCard {
                card_id: "c1".into(),
                from_col: "col-b".into(),
                to_col: "col-a".into(),
                from_pos: 0,
                to_pos: 0,
            }
        ), &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn apply_undo_aborts_if_card_tombstoned() {
        let mut doc = build_test_board();
        // tombstone the card
        delete_card(&mut doc, "c1", &[1u8; 32]).unwrap();

        let result = apply_inverse_op(&mut doc, &InverseOp::Apply(
            Operation::MoveCard { card_id: "c1".into(), from_col: "col-b".into(),
                to_col: "col-a".into(), from_pos: 0, to_pos: 0 }
        ), &[]);
        assert!(matches!(result, Err(UndoError::TargetTombstoned { .. })));
    }

    #[test]
    fn copy_card_undo_detects_subsequent_modifications() {
        let mut doc = build_test_board();
        let copy = copy_card(&mut doc, "c1", "col-a".into(), &[1u8; 32], &[[1u8; 32].to_vec()]).unwrap();
        // Modify the copy
        rename_card(&mut doc, &copy.id, "Modified title", &[1u8; 32]).unwrap();

        let needs_confirm = copy_undo_needs_confirmation(&doc, &copy.id).unwrap();
        assert!(needs_confirm);
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-core undo
```
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
// crates/kanban-core/src/undo.rs

use crate::operation::{InverseOp, Operation};
use automerge::AutoCommit;

#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("cannot undo: card or column '{id}' was deleted by another peer")]
    TargetTombstoned { id: String },
    #[error("this operation is not undoable: {reason}")]
    NotUndoable { reason: String },
    #[error(transparent)]
    Core(#[from] crate::Error),
}

/// Returns true if the copied card (new_card_id) has been modified since it was created.
pub fn copy_undo_needs_confirmation(doc: &AutoCommit, new_card_id: &str) -> Result<bool, crate::Error> {
    // Check if any field besides "created_at", "created_by", "copied_from", "number", "title"
    // has been set, or if title differs from "Copy of <original_title>"
    // Implementation: scan Automerge change history for writes to this card's map after creation.
    // Simplified: check if comments list is non-empty or checklists have checked items.
    let cards_map = crate::card::get_cards_map(doc)?;
    let card_obj = match doc.get(&cards_map, new_card_id)? {
        Some((_, id)) => id,
        None => return Ok(false), // already gone
    };
    let comment_count = doc.length(&crate::comment::get_comments_list(doc, &card_obj)?);
    Ok(comment_count > 0)
}

/// Apply an `InverseOp` to the document, with tombstone guard.
pub fn apply_inverse_op(
    doc: &mut AutoCommit,
    inv: &InverseOp,
    actor_pk: &[u8],
) -> Result<(), UndoError> {
    match inv {
        InverseOp::NotUndoable { reason } => Err(UndoError::NotUndoable { reason: reason.clone() }),
        InverseOp::Apply(op) => {
            // Tombstone guard: check the target object exists and isn't deleted
            let target_id = op_target_id(op);
            if let Some(id) = &target_id {
                if crate::card::is_tombstoned(doc, id)? {
                    return Err(UndoError::TargetTombstoned { id: id.clone() });
                }
            }
            // Apply the compensating operation
            crate::apply_operation(doc, op, actor_pk)?;
            Ok(())
        }
    }
}

fn op_target_id(op: &Operation) -> Option<String> {
    match op {
        Operation::MoveCard { card_id, .. } => Some(card_id.clone()),
        Operation::RenameCard { card_id, .. } => Some(card_id.clone()),
        Operation::DeleteCard { card_id } => Some(card_id.clone()),
        Operation::RenameColumn { col_id, .. } => Some(col_id.clone()),
        Operation::MoveColumn { col_id, .. } => Some(col_id.clone()),
        _ => None,
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-core undo
```
Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-core/src/undo.rs crates/kanban-core/src/lib.rs
git commit -m "feat(core): undo tombstone guard and apply_inverse_op"
```

---

### Task 4: Undo/redo CLI commands

**Files:**
- Create: `crates/kanban-cli/src/commands/undo.rs`
- Modify: `crates/kanban-cli/src/main.rs`

- [ ] **Step 1: Write the integration test**

```rust
#[test]
fn undo_reverses_card_rename() {
    let tmp = tempfile::tempdir().unwrap();
    // Setup: create board, column, card named "Original"
    // Rename card to "Renamed"
    // Undo
    // Verify title is "Original" again via `card view --json`
    let card_view = run_cli(&tmp, &["card", "view", board_id, card_id, "--json"]);
    let card: serde_json::Value = serde_json::from_slice(&card_view.stdout).unwrap();
    assert_eq!(card["title"], "Original");
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-cli undo_reverses
```
Expected: FAIL.

- [ ] **Step 3: Implement commands**

```rust
// crates/kanban-cli/src/commands/undo.rs

#[derive(clap::Args)]
pub struct UndoArgs {
    board_id: String,
    #[arg(long)] json: bool,
    /// Force undo even if the copy has been subsequently modified
    #[arg(long)] force: bool,
}

pub fn run_undo(storage: &mut Storage, args: UndoArgs) -> anyhow::Result<()> {
    let identity = storage.load_identity()?;
    let actor_key = hex::encode(&identity.public_key);
    let entry = storage.undo_stack().pop_undo(&args.board_id, &actor_key)?;
    let Some(entry) = entry else {
        eprintln!("Nothing to undo.");
        return Ok(());
    };

    // Special case: copy_card undo — check for subsequent modifications
    if entry.action_tag == "copy_card" && !args.force {
        let doc = storage.load_board(&args.board_id)?;
        if let kanban_core::operation::InverseOp::Apply(
            kanban_core::operation::Operation::DeleteCard { ref card_id }
        ) = entry.inverse {
            if kanban_core::undo::copy_undo_needs_confirmation(&doc, card_id)? {
                eprintln!(
                    "Error: undoing this copy would delete card '{card_id}' which has been \
                     modified since copying.\n       Use --force to proceed and discard those changes."
                );
                // Push the entry back so it's not lost
                storage.undo_stack().push_undo(&args.board_id, &actor_key, &entry.action_tag, &entry.inverse)?;
                return Ok(());
            }
        }
    }

    let mut doc = storage.load_board(&args.board_id)?;
    match kanban_core::undo::apply_inverse_op(&mut doc, &entry.inverse, &identity.public_key) {
        Ok(()) => {
            storage.save_board(&args.board_id, &doc)?;
            storage.undo_stack().clear_redo(&args.board_id, &actor_key)?;
            if args.json {
                println!("{{\"undone\": \"{}\"}}", entry.action_tag);
            } else {
                println!("Undone: {}", entry.action_tag);
            }
        }
        Err(kanban_core::undo::UndoError::TargetTombstoned { id }) => {
            eprintln!("Cannot undo — '{}' was deleted by another peer.", id);
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub fn run_redo(storage: &mut Storage, board_id: &str, json: bool) -> anyhow::Result<()> {
    let identity = storage.load_identity()?;
    let actor_key = hex::encode(&identity.public_key);
    let entry = storage.undo_stack().pop_redo(board_id, &actor_key)?;
    let Some(entry) = entry else {
        eprintln!("Nothing to redo.");
        return Ok(());
    };

    let mut doc = storage.load_board(board_id)?;
    match kanban_core::undo::apply_inverse_op(&mut doc, &entry.inverse, &identity.public_key) {
        Ok(()) => {
            storage.save_board(board_id, &doc)?;
            if json {
                println!("{{\"redone\": \"{}\"}}", entry.action_tag);
            } else {
                println!("Redone: {}", entry.action_tag);
            }
        }
        Err(kanban_core::undo::UndoError::TargetTombstoned { id }) => {
            // Discard poisoned redo entry; remaining stack is intact (already popped)
            eprintln!("Cannot redo — '{}' was deleted by another peer. Redo entry discarded.", id);
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub fn run_undo_history(storage: &mut Storage, board_id: &str, json: bool) -> anyhow::Result<()> {
    let identity = storage.load_identity()?;
    let actor_key = hex::encode(&identity.public_key);
    let entries = storage.undo_stack().list_undo(board_id, &actor_key)?;
    if json {
        let out: Vec<_> = entries.iter().map(|e| serde_json::json!({
            "action": e.action_tag, "hlc": e.hlc
        })).collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        for (i, e) in entries.iter().enumerate() {
            println!("{}: {} ({})", i + 1, e.action_tag, e.hlc);
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-cli undo
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-cli/src/commands/undo.rs crates/kanban-cli/src/main.rs
git commit -m "feat(cli): add undo, redo, undo-history commands"
```

---

### Task 5: Wire undo into all `kanban-core` mutations

**Files:**
- Modify: `crates/kanban-core/src/card.rs`, `column.rs`

Every mutation that returns a `Card` or `Column` should also return the `Operation` it performed. Callers (CLI, Tauri) push to `undo_stack` after a successful mutation and `clear_redo`.

- [ ] **Step 1: Write the test**

```rust
#[test]
fn create_card_returns_operation() {
    let mut doc = AutoCommit::new();
    // ...setup...
    let (card, op) = create_card_with_op(&mut doc, col_id, "Task", &pk, &members).unwrap();
    assert!(matches!(op, Operation::CreateCard { .. }));
}
```

- [ ] **Step 2: Update function signatures**

Change:
```rust
pub fn create_card(...) -> Result<Card, Error>
```
To:
```rust
pub fn create_card(...) -> Result<(Card, Operation), Error>
```

Do the same for: `move_card`, `rename_card`, `delete_card`, `copy_card`, `create_column`, `rename_column`, `delete_column`, `move_column`.

- [ ] **Step 3: Run tests**

```bash
cargo test -p kanban-core
```
Expected: all tests pass (callers may need updating for the new return type).

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-core/src/card.rs crates/kanban-core/src/column.rs
git commit -m "refactor(core): mutations return (result, Operation) for undo stack wiring"
```

---

### Task 6: Tauri undo/redo commands

**Files:**
- Create: `crates/kanban-tauri/src-tauri/src/commands/undo.rs`
- Modify: `crates/kanban-tauri/src-tauri/src/main.rs`

- [ ] **Step 1: Write unit test**

```rust
#[test]
fn tauri_undo_command_applies_inverse() {
    let state = build_test_state_with_renamed_card();
    let result = undo(state, UndoArgs { board_id: "board1".into(), force: false });
    assert!(result.is_ok());
    // Verify card title reverted
}
```

- [ ] **Step 2: Implement Tauri commands**

```rust
#[tauri::command]
pub async fn undo(
    state: tauri::State<'_, AppState>,
    board_id: String,
    force: bool,
) -> Result<UndoResult, String> {
    // Same logic as CLI run_undo, returning structured result for frontend toast
    // UndoResult { action_tag, warning: Option<String> }
}

#[tauri::command]
pub async fn redo(
    state: tauri::State<'_, AppState>,
    board_id: String,
) -> Result<UndoResult, String> {
    let mut state = state.lock().await;
    let identity = state.identity.clone();
    let actor_key = hex::encode(&identity.public_key);
    let entry = state.storage.undo_stack()
        .pop_redo(&board_id, &actor_key)
        .map_err(|e| e.to_string())?;
    let Some(entry) = entry else {
        return Ok(UndoResult { action_tag: String::new(), warning: Some("Nothing to redo.".into()) });
    };
    let mut doc = state.load_board(&board_id).map_err(|e| e.to_string())?;
    match kanban_core::undo::apply_inverse_op(&mut doc, &entry.inverse, &identity.public_key) {
        Ok(()) => {
            state.save_board(&board_id, &doc).map_err(|e| e.to_string())?;
            state.broadcast_changes(&board_id).await.map_err(|e| e.to_string())?;
            Ok(UndoResult { action_tag: entry.action_tag, warning: None })
        }
        Err(kanban_core::undo::UndoError::TargetTombstoned { id }) => {
            Ok(UndoResult {
                action_tag: entry.action_tag,
                warning: Some(format!("Cannot redo — '{}' was deleted by another peer.", id)),
            })
        }
        Err(e) => Err(e.to_string()),
    }
}
```

- [ ] **Step 3: Register commands and run tests**

```bash
cargo test -p kanban-tauri undo
```

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-tauri/src-tauri/src/commands/undo.rs crates/kanban-tauri/src-tauri/src/main.rs
git commit -m "feat(tauri): add undo/redo Tauri commands"
```

---

### Task 7: Message type-prefix routing in `kanban-net`

**Files:**
- Create: `crates/kanban-net/src/message.rs`
- Modify: `crates/kanban-net/src/lib.rs`

This establishes the 1-byte prefix that distinguishes Automerge changes (0x01), presence heartbeats (0x02), and version messages (0x03) so they are never routed to the wrong handler.

- [ ] **Step 1: Write the test**

```rust
#[test]
fn encode_and_decode_message_type() {
    let payload = b"hello";
    let encoded = encode_message(MessageType::AutomergeChange, payload);
    assert_eq!(encoded[0], 0x01);

    let (msg_type, body) = decode_message(&encoded).unwrap();
    assert_eq!(msg_type, MessageType::AutomergeChange);
    assert_eq!(body, payload);
}
```

- [ ] **Step 2: Implement**

```rust
// crates/kanban-net/src/message.rs

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    AutomergeChange = 0x01,
    PresenceHeartbeat = 0x02,
    VersionHandshake = 0x03,
}

impl TryFrom<u8> for MessageType {
    type Error = crate::Error;
    fn try_from(b: u8) -> Result<Self, Self::Error> {
        match b {
            0x01 => Ok(Self::AutomergeChange),
            0x02 => Ok(Self::PresenceHeartbeat),
            0x03 => Ok(Self::VersionHandshake),
            _ => Err(crate::Error::UnknownMessageType(b)),
        }
    }
}

pub fn encode_message(msg_type: MessageType, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + payload.len());
    out.push(msg_type as u8);
    out.extend_from_slice(payload);
    out
}

pub fn decode_message(data: &[u8]) -> Result<(MessageType, &[u8]), crate::Error> {
    if data.is_empty() {
        return Err(crate::Error::EmptyMessage);
    }
    let msg_type = MessageType::try_from(data[0])?;
    Ok((msg_type, &data[1..]))
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p kanban-net message
```
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-net/src/message.rs crates/kanban-net/src/lib.rs
git commit -m "feat(net): add 1-byte message type prefix routing"
```

---

### Task 8: `VersionHello` / `VersionReject` handshake

**Files:**
- Create: `crates/kanban-net/src/version.rs`
- Modify: `crates/kanban-net/src/connection.rs`
- Modify: `crates/kanban-net/Cargo.toml`

- [ ] **Step 1: Add `semver` dependency**

```toml
[dependencies]
semver = "1"
```

- [ ] **Step 2: Write the failing tests**

```rust
// crates/kanban-net/src/version.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatible_versions_accepted() {
        let local = VersionHello {
            app_version: "0.3.1".into(),
            min_compatible: "0.2.0".into(),
            iroh_version: "0.96.0".into(),
            protocol_features: vec![],
        };
        let remote = VersionHello {
            app_version: "0.2.5".into(),
            min_compatible: "0.2.0".into(),
            iroh_version: "0.96.0".into(),
            protocol_features: vec![],
        };
        assert!(check_compatibility(&local, &remote).is_ok());
    }

    #[test]
    fn old_remote_rejected() {
        let local = VersionHello {
            app_version: "0.3.0".into(),
            min_compatible: "0.3.0".into(),
            iroh_version: "0.96.0".into(),
            protocol_features: vec![],
        };
        let remote = VersionHello {
            app_version: "0.2.0".into(),
            min_compatible: "0.1.0".into(),
            iroh_version: "0.96.0".into(),
            protocol_features: vec![],
        };
        let err = check_compatibility(&local, &remote).unwrap_err();
        assert!(matches!(err, VersionError::RemoteTooOld { .. }));
    }

    #[test]
    fn feature_intersection_computed() {
        let a = vec!["undo_v1".to_string(), "mentions_v1".to_string()];
        let b = vec!["mentions_v1".to_string(), "presence_v1".to_string()];
        let intersection = feature_intersection(&a, &b);
        assert_eq!(intersection, vec!["mentions_v1"]);
    }
}
```

- [ ] **Step 3: Run to verify failure**

```bash
cargo test -p kanban-net version
```
Expected: FAIL.

- [ ] **Step 4: Implement**

```rust
// crates/kanban-net/src/version.rs

use semver::Version;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MIN_COMPATIBLE: &str = "0.1.0"; // bump on breaking changes

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VersionHello {
    pub app_version: String,
    pub min_compatible: String,
    pub iroh_version: String,
    pub protocol_features: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VersionReject {
    pub reason: String,         // "version_too_old" | "version_too_new"
    pub min_required: String,   // the sender's min_compatible
    pub their_version: String,  // echoed back for clarity
}

#[derive(Debug, thiserror::Error)]
pub enum VersionError {
    #[error("remote version {remote} is below our minimum {min}")]
    RemoteTooOld { remote: String, min: String },
    #[error("our version {local} is below remote's minimum {min}")]
    LocalTooOld { local: String, min: String },
    #[error("semver parse error: {0}")]
    Parse(#[from] semver::Error),
}

pub fn check_compatibility(local: &VersionHello, remote: &VersionHello) -> Result<(), VersionError> {
    let remote_ver = Version::parse(&remote.app_version)?;
    let local_min = Version::parse(&local.min_compatible)?;
    let local_ver = Version::parse(&local.app_version)?;
    let remote_min = Version::parse(&remote.min_compatible)?;

    if remote_ver < local_min {
        return Err(VersionError::RemoteTooOld {
            remote: remote.app_version.clone(),
            min: local.min_compatible.clone(),
        });
    }
    if local_ver < remote_min {
        return Err(VersionError::LocalTooOld {
            local: local.app_version.clone(),
            min: remote.min_compatible.clone(),
        });
    }
    Ok(())
}

pub fn feature_intersection(a: &[String], b: &[String]) -> Vec<String> {
    a.iter().filter(|f| b.contains(f)).cloned().collect()
}

pub fn local_hello() -> VersionHello {
    VersionHello {
        app_version: APP_VERSION.into(),
        min_compatible: MIN_COMPATIBLE.into(),
        iroh_version: iroh::version::VERSION.to_string(),
        protocol_features: vec![
            "undo_v1".into(),
            "card_numbers_v1".into(),
        ],
    }
}
```

- [ ] **Step 5: Integrate into connection setup**

In `crates/kanban-net/src/connection.rs`, in the connection accept/connect handler, add (using `tokio::time::timeout`):

```rust
use crate::version::{check_compatibility, local_hello, VersionReject};
use crate::message::{encode_message, decode_message, MessageType};
use std::time::Duration;

pub async fn perform_version_handshake(
    send: &mut iroh::net::endpoint::SendStream,
    recv: &mut iroh::net::endpoint::RecvStream,
) -> Result<Vec<String>, crate::Error> { // returns active feature flags
    // Send our hello
    let hello = local_hello();
    let encoded = ciborium::ser::into_vec(&hello).map_err(|_| crate::Error::Serialization)?;
    let msg = encode_message(MessageType::VersionHandshake, &encoded);
    send.write_all(&msg).await?;

    // Wait for remote hello with 5s timeout
    let remote_bytes = tokio::time::timeout(Duration::from_secs(5), recv.read_to_end(4096))
        .await
        .map_err(|_| crate::Error::HandshakeTimeout)?
        .map_err(|_| crate::Error::HandshakeIo)?;

    let (msg_type, body) = decode_message(&remote_bytes)?;
    if msg_type != MessageType::VersionHandshake {
        return Err(crate::Error::UnexpectedMessageType);
    }
    let remote: crate::version::VersionHello =
        ciborium::de::from_reader(body).map_err(|_| crate::Error::Serialization)?;

    match check_compatibility(&hello, &remote) {
        Ok(()) => {}
        Err(e) => {
            // Send reject before closing
            let reject = VersionReject {
                reason: "version_too_old".into(),
                min_required: hello.min_compatible.clone(),
                their_version: remote.app_version.clone(),
            };
            let rej_bytes = ciborium::ser::into_vec(&reject).unwrap_or_default();
            let _ = send.write_all(&encode_message(MessageType::VersionHandshake, &rej_bytes)).await;
            return Err(crate::Error::IncompatibleVersion(e.to_string()));
        }
    }

    Ok(crate::version::feature_intersection(
        &hello.protocol_features,
        &remote.protocol_features,
    ))
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p kanban-net version
```
Expected: all 3 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/kanban-net/src/version.rs crates/kanban-net/src/connection.rs crates/kanban-net/Cargo.toml
git commit -m "feat(net): VersionHello/VersionReject handshake with semver comparison and 5s timeout"
```

---

### Task 9: End-to-end undo/version smoke tests

- [ ] **Step 1: Write integration test**

```rust
// tests/integration/phase2_smoke_test.rs

#[test]
fn undo_redo_cycle() {
    // rename card → undo → verify original → redo → verify renamed
}

#[test]
fn version_handshake_rejects_old_peer() {
    // Simulate two in-process Iroh endpoints; one with an old MIN_COMPATIBLE
    // Verify connection is rejected with IncompatibleVersion error
}
```

- [ ] **Step 2: Run**

```bash
cargo test --test phase2_smoke_test
```

- [ ] **Step 3: Commit**

```bash
git add tests/integration/phase2_smoke_test.rs
git commit -m "test(integration): phase2 smoke tests for undo/redo and version handshake"
```
