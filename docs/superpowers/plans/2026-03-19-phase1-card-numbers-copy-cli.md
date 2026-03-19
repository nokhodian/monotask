# Phase 1 Feature Additions: Card Numbers, Card Copy, Comment/Checklist CLI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add actor-scoped human-readable card numbers, card copy/duplicate, and CLI commands for comments and checklists — all local operations, no networking changes required.

**Architecture:** Each card gets a `prefix-seq` display number (e.g. `a7f3-1`) where the prefix is derived from the creating actor's public key and the seq is a per-actor Automerge counter. A local SQLite index maps card numbers to UUIDs. Card copy generates a new card with selected fields copied. Comment and checklist commands are thin CLI wrappers over existing `kanban-core` operations.

**Tech Stack:** Rust 2021, `automerge = "0.5"`, `rusqlite`, `clap` v4, `base32`, `regex`

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| Create | `crates/kanban-core/src/card_number.rs` | `CardNumber` type, actor prefix, parse/display |
| Modify | `crates/kanban-core/src/lib.rs` | Re-export `card_number` module |
| Modify | `crates/kanban-core/src/card.rs` | Add `number` field; assign in `create_card`; `copy_card` op |
| Create | `crates/kanban-core/src/migration.rs` | One-time number assignment for pre-existing cards |
| Create | `crates/kanban-storage/src/card_number.rs` | `card_number_index` CRUD, `sync_card_number_index`, `resolve_card_ref` |
| Modify | `crates/kanban-storage/src/schema.rs` | Add `card_number_index` migration |
| Modify | `crates/kanban-storage/src/lib.rs` | Re-export `card_number` module; call sync after every Automerge merge |
| Create | `crates/kanban-cli/src/commands/comment.rs` | `comment add`, `comment list`, `comment delete` |
| Create | `crates/kanban-cli/src/commands/checklist.rs` | Full checklist + checklist-item CRUD |
| Modify | `crates/kanban-cli/src/main.rs` | Register new subcommands |
| Modify | `crates/kanban-tauri/src-tauri/src/commands/card.rs` | `copy_card` Tauri command |

---

### Task 1: `CardNumber` type and actor prefix

**Files:**
- Create: `crates/kanban-core/src/card_number.rs`
- Modify: `crates/kanban-core/Cargo.toml` (add `base32`, `regex` deps)

- [ ] **Step 1: Add dependencies**

In `crates/kanban-core/Cargo.toml`:
```toml
[dependencies]
base32 = "0.4"
regex = "1"
```

- [ ] **Step 2: Write the failing tests**

Create `crates/kanban-core/src/card_number.rs`:
```rust
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CardNumber {
    pub prefix: String,
    pub seq: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CardNumberError {
    #[error("invalid card number format; expected <prefix>-<integer>, e.g. 'a7f3-1'")]
    InvalidFormat,
}

impl CardNumber {
    pub fn new(prefix: impl Into<String>, seq: u64) -> Self {
        Self { prefix: prefix.into(), seq }
    }

    pub fn to_display(&self) -> String {
        format!("{}-{}", self.prefix, self.seq)
    }
}

impl FromStr for CardNumber {
    type Err = CardNumberError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Accepts 4–8 lowercase alphanumeric prefix, dash, positive integer
        let re = regex::Regex::new(r"^([a-z0-9]{4,8})-(\d+)$").unwrap();
        let caps = re.captures(s).ok_or(CardNumberError::InvalidFormat)?;
        let prefix = caps[1].to_string();
        let seq: u64 = caps[2].parse().map_err(|_| CardNumberError::InvalidFormat)?;
        Ok(CardNumber { prefix, seq })
    }
}

/// Derive the actor's display prefix for a given board.
/// Uses 4 chars normally; extends to 8 if another board member shares the same 4-char prefix.
pub fn actor_prefix(pubkey_bytes: &[u8], all_member_pubkeys: &[Vec<u8>]) -> String {
    let encoded = base32::encode(
        base32::Alphabet::RFC4648 { padding: false },
        pubkey_bytes,
    )
    .to_lowercase();

    let prefix4 = &encoded[..4];
    let collision = all_member_pubkeys
        .iter()
        .filter(|pk| pk.as_slice() != pubkey_bytes)
        .any(|pk| {
            let other = base32::encode(base32::Alphabet::RFC4648 { padding: false }, pk)
                .to_lowercase();
            other.starts_with(prefix4)
        });

    if collision {
        encoded[..8].to_string()
    } else {
        prefix4.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_card_number() {
        let n: CardNumber = "a7f3-42".parse().unwrap();
        assert_eq!(n.prefix, "a7f3");
        assert_eq!(n.seq, 42);
    }

    #[test]
    fn roundtrip_display() {
        let n = CardNumber::new("a7f3", 1);
        let s = n.to_display();
        let parsed: CardNumber = s.parse().unwrap();
        assert_eq!(parsed, n);
    }

    #[test]
    fn reject_invalid_format() {
        assert!("42".parse::<CardNumber>().is_err());
        assert!("toolongprefix-1".parse::<CardNumber>().is_err());
        assert!("a7f3-".parse::<CardNumber>().is_err());
    }

    #[test]
    fn actor_prefix_no_collision() {
        let pk = vec![1u8; 32];
        let others = vec![vec![2u8; 32]];
        let p = actor_prefix(&pk, &others);
        assert_eq!(p.len(), 4);
    }

    #[test]
    fn actor_prefix_collision_extends_to_8() {
        // Craft two keys that share the same 4-char base32 prefix
        // The easiest way: use the same first byte pattern
        // We'll just use a mock — verify the function extends when equal
        let pk = vec![0u8; 32];
        // Another key that base32-encodes to the same 4-char prefix as pk
        // All-zero bytes base32-encode to "AAAAAAA..." so prefix4 = "aaaa"
        let other = vec![0u8; 32]; // same prefix
        let p = actor_prefix(&pk, &[other]);
        assert_eq!(p.len(), 8);
    }
}
```

- [ ] **Step 3: Run failing tests**

```bash
cd crates/kanban-core && cargo test card_number
```
Expected: compile error (module not declared in `lib.rs`)

- [ ] **Step 4: Declare module in lib.rs**

In `crates/kanban-core/src/lib.rs`, add:
```rust
pub mod card_number;
```

- [ ] **Step 5: Run tests again**

```bash
cargo test -p kanban-core card_number
```
Expected: all 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/kanban-core/src/card_number.rs crates/kanban-core/src/lib.rs crates/kanban-core/Cargo.toml
git commit -m "feat(core): add CardNumber type and actor_prefix derivation"
```

---

### Task 2: Assign card numbers in `create_card`

**Files:**
- Modify: `crates/kanban-core/src/card.rs`

The card struct already has fields for title, description, etc. This task adds the `number` field and wires it into creation.

- [ ] **Step 1: Write the failing test**

In `crates/kanban-core/src/card.rs`, add to the `#[cfg(test)]` block:
```rust
#[test]
fn create_card_assigns_number() {
    let mut doc = automerge::AutoCommit::new();
    let actor_pk = vec![1u8; 32];
    let members = vec![actor_pk.clone()];
    let board_id = "board1";

    // Create the board structure
    init_board_doc(&mut doc, board_id, &actor_pk).unwrap();

    let col_id = create_column(&mut doc, "To Do").unwrap();
    let card = create_card(&mut doc, col_id, "My Task", &actor_pk, &members).unwrap();

    assert!(card.number.is_some());
    let num = card.number.unwrap();
    assert_eq!(num.seq, 1);
    assert!(!num.prefix.is_empty());
}

#[test]
fn sequential_cards_have_increasing_seq() {
    let mut doc = automerge::AutoCommit::new();
    let actor_pk = vec![1u8; 32];
    let members = vec![actor_pk.clone()];
    init_board_doc(&mut doc, "board1", &actor_pk).unwrap();
    let col_id = create_column(&mut doc, "To Do").unwrap();

    let c1 = create_card(&mut doc, col_id.clone(), "Task 1", &actor_pk, &members).unwrap();
    let c2 = create_card(&mut doc, col_id, "Task 2", &actor_pk, &members).unwrap();

    assert_eq!(c1.number.unwrap().seq, 1);
    assert_eq!(c2.number.unwrap().seq, 2);
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-core create_card_assigns_number
```
Expected: FAIL — `number` field doesn't exist yet.

- [ ] **Step 3: Add `number` field to Card struct and implement assignment**

In `crates/kanban-core/src/card.rs`:
```rust
use crate::card_number::{actor_prefix, CardNumber};
use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, ScalarValue, ROOT, transaction::Transactable};

// Add to Card struct:
pub struct Card {
    pub id: String,
    pub number: Option<CardNumber>,
    pub title: String,
    pub description: String,
    // ... existing fields ...
}

/// Increment (or initialize) the per-actor card sequence counter in the Automerge doc.
/// Returns the next CardNumber for this actor.
fn assign_next_card_number(
    doc: &mut AutoCommit,
    actor_pk: &[u8],
    all_members: &[Vec<u8>],
) -> Result<CardNumber, crate::Error> {
    let actor_key = hex::encode(actor_pk); // stable string key for this actor

    // Ensure actor_card_seq map exists at root
    let seq_map: ObjId = match doc.get(ROOT, "actor_card_seq")? {
        Some((automerge::Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(ROOT, "actor_card_seq", ObjType::Map)?,
    };

    let next_seq: u64 = match doc.get(&seq_map, &actor_key)? {
        Some((automerge::Value::Scalar(s), _)) => {
            if let ScalarValue::Counter(c) = s.as_ref() {
                doc.increment(&seq_map, &actor_key, 1)?;
                (*c + 1) as u64
            } else {
                return Err(crate::Error::InvalidDocument("actor_card_seq entry is not a counter".into()));
            }
        }
        _ => {
            doc.put(&seq_map, &actor_key, ScalarValue::Counter(1))?;
            1
        }
    };

    let prefix = actor_prefix(actor_pk, all_members);
    Ok(CardNumber::new(prefix, next_seq))
}

pub fn create_card(
    doc: &mut AutoCommit,
    col_id: ObjId,
    title: &str,
    actor_pk: &[u8],
    all_members: &[Vec<u8>],
) -> Result<Card, crate::Error> {
    let number = assign_next_card_number(doc, actor_pk, all_members)?;
    // ... rest of card creation, store number.to_display() in doc ...
    let card_id = uuid::Uuid::new_v4().to_string();
    let cards_map = get_or_create_cards_map(doc)?;
    let card_obj = doc.put_object(&cards_map, &card_id, ObjType::Map)?;
    doc.put(&card_obj, "title", title)?;
    doc.put(&card_obj, "number", number.to_display())?;
    doc.put(&card_obj, "created_by", hex::encode(actor_pk))?;
    // ... add to column card_ids list ...

    Ok(Card {
        id: card_id,
        number: Some(number),
        title: title.to_string(),
        description: String::new(),
    })
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-core create_card
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-core/src/card.rs
git commit -m "feat(core): assign actor-scoped card numbers on create_card"
```

---

### Task 3: `card_number_index` in SQLite storage

**Files:**
- Modify: `crates/kanban-storage/src/schema.rs`
- Create: `crates/kanban-storage/src/card_number.rs`
- Modify: `crates/kanban-storage/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In `crates/kanban-storage/src/card_number.rs` (create file):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::schema::run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn upsert_and_resolve_by_number() {
        let conn = test_db();
        upsert_card_number(&conn, "board1", "card-uuid-1", "a7f3-1").unwrap();
        let uuid = resolve_card_ref(&conn, "board1", "a7f3-1").unwrap();
        assert_eq!(uuid, "card-uuid-1");
    }

    #[test]
    fn resolve_uuid_passthrough() {
        let conn = test_db();
        // A UUID-shaped ref that doesn't match the card number pattern passes through
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let result = resolve_card_ref(&conn, "board1", uuid).unwrap();
        assert_eq!(result, uuid);
    }

    #[test]
    fn upsert_is_idempotent() {
        let conn = test_db();
        upsert_card_number(&conn, "board1", "card-uuid-1", "a7f3-1").unwrap();
        upsert_card_number(&conn, "board1", "card-uuid-1", "a7f3-1").unwrap(); // no error
        let uuid = resolve_card_ref(&conn, "board1", "a7f3-1").unwrap();
        assert_eq!(uuid, "card-uuid-1");
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-storage card_number
```
Expected: FAIL — table and functions don't exist.

- [ ] **Step 3: Add migration**

In `crates/kanban-storage/src/schema.rs`, add a new migration:
```sql
-- Migration N: card_number_index
CREATE TABLE IF NOT EXISTS card_number_index (
    board_id   TEXT NOT NULL,
    card_id    TEXT NOT NULL,
    number     TEXT NOT NULL,
    PRIMARY KEY (board_id, card_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_card_number_lookup
    ON card_number_index (board_id, number);
```

- [ ] **Step 4: Implement the functions**

In `crates/kanban-storage/src/card_number.rs`:
```rust
use rusqlite::{Connection, params};
use kanban_core::card_number::CardNumber;

/// Upsert a (board_id, card_id, number) row into the index.
pub fn upsert_card_number(
    conn: &Connection,
    board_id: &str,
    card_id: &str,
    number: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO card_number_index (board_id, card_id, number)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(board_id, card_id) DO UPDATE SET number = excluded.number",
        params![board_id, card_id, number],
    )?;
    Ok(())
}

/// Resolve a card reference string to a UUID.
///
/// If `card_ref` matches the `<prefix>-<seq>` pattern, look it up in the index.
/// Otherwise treat it as a UUID and return it unchanged.
pub fn resolve_card_ref(
    conn: &Connection,
    board_id: &str,
    card_ref: &str,
) -> Result<String, crate::Error> {
    // Try parsing as card number
    if card_ref.parse::<CardNumber>().is_ok() {
        let result: Option<String> = conn
            .query_row(
                "SELECT card_id FROM card_number_index
                 WHERE board_id = ?1 AND number = ?2",
                params![board_id, card_ref],
                |row| row.get(0),
            )
            .optional()?;
        result.ok_or_else(|| crate::Error::NotFound(format!("card {card_ref} not found in board {board_id}")))
    } else {
        // UUID passthrough
        Ok(card_ref.to_string())
    }
}

/// Sync the index for a list of (card_id, number) pairs from the Automerge document.
/// Called after every Automerge merge (local or peer-delivered).
pub fn sync_card_number_index(
    conn: &Connection,
    board_id: &str,
    cards: &[(String, String)], // (card_id, number)
) -> rusqlite::Result<()> {
    for (card_id, number) in cards {
        upsert_card_number(conn, board_id, card_id, number)?;
    }
    Ok(())
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p kanban-storage card_number
```
Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/kanban-storage/src/card_number.rs crates/kanban-storage/src/schema.rs crates/kanban-storage/src/lib.rs
git commit -m "feat(storage): add card_number_index table and sync functions"
```

---

### Task 4: One-time migration for pre-existing cards

**Files:**
- Create: `crates/kanban-core/src/migration.rs`
- Modify: `crates/kanban-core/src/lib.rs`

This runs at app startup for boards that existed before card numbers were introduced. Each actor only migrates their own cards (cards where `created_by == local_pubkey`).

- [ ] **Step 1: Write the failing test**

In `crates/kanban-core/src/migration.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use automerge::AutoCommit;

    #[test]
    fn migrate_assigns_numbers_only_to_own_cards() {
        let mut doc = AutoCommit::new();
        let actor_pk = vec![1u8; 32];
        let other_pk = vec![2u8; 32];
        let members = vec![actor_pk.clone(), other_pk.clone()];

        // Simulate two existing cards: one owned by actor, one by other
        let own_card_id = create_card_without_number(&mut doc, "Task A", &actor_pk);
        let other_card_id = create_card_without_number(&mut doc, "Task B", &other_pk);

        let migrated = assign_numbers_for_actor(&mut doc, &actor_pk, &members).unwrap();

        assert_eq!(migrated.len(), 1);
        assert_eq!(migrated[0].0, own_card_id);
        // Other's card has no number yet
        assert!(get_card_number(&doc, &other_card_id).is_none());
    }
}
```

- [ ] **Step 2: Implement migration**

```rust
/// Assigns card numbers to all cards owned by `actor_pk` that don't yet have a number.
/// Returns list of (card_id, number_string) pairs that were assigned.
pub fn assign_numbers_for_actor(
    doc: &mut AutoCommit,
    actor_pk: &[u8],
    all_members: &[Vec<u8>],
) -> Result<Vec<(String, String)>, crate::Error> {
    let actor_key = hex::encode(actor_pk);
    let cards_map = get_cards_map(doc)?;
    let mut assigned = Vec::new();

    // Collect cards owned by this actor without a number, sorted by HLC
    let mut owned_unnumbered: Vec<(String, String)> = doc // (card_id, hlc)
        .map_range(&cards_map, ..)
        .filter_map(|(card_id, _, card_obj)| {
            let created_by = doc.get(&card_obj, "created_by").ok()
                .flatten().map(|(v, _)| v.to_str().map(|s| s.to_string()))
                .flatten();
            let number = doc.get(&card_obj, "number").ok().flatten();
            if created_by.as_deref() == Some(&actor_key) && number.is_none() {
                let hlc = doc.get(&card_obj, "created_at").ok().flatten()
                    .map(|(v, _)| v.to_str().map(|s| s.to_string())).flatten()
                    .unwrap_or_default();
                Some((card_id.to_string(), hlc))
            } else {
                None
            }
        })
        .collect();

    // Sort by HLC so numbers are assigned in causal order
    owned_unnumbered.sort_by(|a, b| a.1.cmp(&b.1));

    for (card_id, _) in &owned_unnumbered {
        let number = crate::card::assign_next_card_number(doc, actor_pk, all_members)?;
        let num_str = number.to_display();
        // Write number into the card's Automerge map
        let cards_map = get_cards_map(doc)?;
        let card_obj = doc.get(&cards_map, card_id)?
            .ok_or_else(|| crate::Error::NotFound(card_id.clone()))?.1;
        doc.put(&card_obj, "number", num_str.clone())?;
        assigned.push((card_id.clone(), num_str));
    }

    Ok(assigned)
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p kanban-core migration
```
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-core/src/migration.rs crates/kanban-core/src/lib.rs
git commit -m "feat(core): one-time card number migration for pre-existing cards"
```

---

### Task 5: `copy_card` operation

**Files:**
- Modify: `crates/kanban-core/src/card.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn copy_card_produces_new_card_with_fresh_fields() {
    let mut doc = AutoCommit::new();
    let actor_pk = vec![1u8; 32];
    let members = vec![actor_pk.clone()];
    init_board_doc(&mut doc, "board1", &actor_pk).unwrap();
    let col_id = create_column(&mut doc, "To Do").unwrap();
    let original = create_card(&mut doc, col_id.clone(), "Deploy API", &actor_pk, &members).unwrap();

    let copy = copy_card(&mut doc, &original.id, col_id, &actor_pk, &members).unwrap();

    assert_ne!(copy.id, original.id);
    assert_eq!(copy.title, "Copy of Deploy API");
    assert_eq!(copy.number.unwrap().seq, 2); // seq incremented
    assert_eq!(copy.assignees, vec![] as Vec<String>);
    assert_eq!(copy.comments, vec![] as Vec<String>);
    assert_eq!(copy.copied_from, Some(original.id.clone()));
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-core copy_card
```
Expected: FAIL — `copy_card` not defined.

- [ ] **Step 3: Implement `copy_card`**

```rust
pub fn copy_card(
    doc: &mut AutoCommit,
    source_card_id: &str,
    target_col_id: ObjId,
    actor_pk: &[u8],
    all_members: &[Vec<u8>],
) -> Result<Card, crate::Error> {
    let cards_map = get_cards_map(doc)?;
    let src_obj = doc.get(&cards_map, source_card_id)?
        .ok_or_else(|| crate::Error::NotFound(source_card_id.to_string()))?.1;

    // Read fields to copy
    let title = get_string(doc, &src_obj, "title")?
        .map(|t| format!("Copy of {}", t))
        .unwrap_or_else(|| "Copy of card".to_string());
    let description = get_string(doc, &src_obj, "description")?.unwrap_or_default();

    // Assign new number
    let number = assign_next_card_number(doc, actor_pk, all_members)?;
    let new_card_id = uuid::Uuid::new_v4().to_string();
    let hlc = crate::clock::now(); // HLC timestamp

    let new_card_obj = doc.put_object(&cards_map, &new_card_id, ObjType::Map)?;
    doc.put(&new_card_obj, "title", title.as_str())?;
    doc.put(&new_card_obj, "description", description.as_str())?;
    doc.put(&new_card_obj, "number", number.to_display())?;
    doc.put(&new_card_obj, "created_by", hex::encode(actor_pk))?;
    doc.put(&new_card_obj, "created_at", hlc.to_string())?;
    doc.put(&new_card_obj, "copied_from", source_card_id)?;
    // Copy labels list
    copy_list(doc, &src_obj, &new_card_obj, "labels")?;
    // Copy checklists (reset checked = false on all items)
    copy_checklists_reset(doc, &src_obj, &new_card_obj)?;

    // Insert at end of target column
    append_card_to_column(doc, &target_col_id, &new_card_id)?;

    Ok(Card {
        id: new_card_id,
        number: Some(number),
        title,
        description,
        assignees: vec![],
        comments: vec![],
        copied_from: Some(source_card_id.to_string()),
        ..Default::default()
    })
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-core copy_card
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-core/src/card.rs
git commit -m "feat(core): implement copy_card operation"
```

---

### Task 6: Comment CLI commands

**Files:**
- Create: `crates/kanban-cli/src/commands/comment.rs`
- Modify: `crates/kanban-cli/src/main.rs`

- [ ] **Step 1: Write the integration test**

In `tests/cli_comment_test.rs`:
```rust
// Integration test: run CLI binary and check output
#[test]
fn comment_add_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_kanban-cli");

    // Init
    std::process::Command::new(bin)
        .args(["init", "--data-dir", tmp.path().to_str().unwrap()])
        .status().unwrap();

    // Create board + column + card
    let board_out = std::process::Command::new(bin)
        .args(["--data-dir", tmp.path().to_str().unwrap(), "board", "create", "TestBoard", "--json"])
        .output().unwrap();
    let board: serde_json::Value = serde_json::from_slice(&board_out.stdout).unwrap();
    let board_id = board["id"].as_str().unwrap();

    // ... (similar for column, card) ...

    // Add comment
    let add_out = std::process::Command::new(bin)
        .args(["--data-dir", tmp.path().to_str().unwrap(),
               "card", "comment", "add", board_id, card_id, "Hello world", "--json"])
        .output().unwrap();
    assert!(add_out.status.success());
    let result: serde_json::Value = serde_json::from_slice(&add_out.stdout).unwrap();
    assert!(result["id"].is_string());

    // List comments
    let list_out = std::process::Command::new(bin)
        .args(["--data-dir", tmp.path().to_str().unwrap(),
               "card", "comment", "list", board_id, card_id, "--json"])
        .output().unwrap();
    let comments: Vec<serde_json::Value> = serde_json::from_slice(&list_out.stdout).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["text"], "Hello world");
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-cli cli_comment_test
```
Expected: FAIL — subcommand not defined.

- [ ] **Step 3: Implement comment commands**

Create `crates/kanban-cli/src/commands/comment.rs`:
```rust
use clap::{Args, Subcommand};
use kanban_core::comment::{add_comment, delete_comment, list_comments};
use kanban_storage::Storage;

#[derive(Args)]
pub struct CommentArgs {
    #[command(subcommand)]
    pub command: CommentCommand,
}

#[derive(Subcommand)]
pub enum CommentCommand {
    /// Add a comment to a card
    Add {
        board_id: String,
        card_id: String,
        text: String,
        #[arg(long)] json: bool,
    },
    /// List comments on a card
    List {
        board_id: String,
        card_id: String,
        #[arg(long)] json: bool,
    },
    /// Delete a comment (tombstone)
    Delete {
        board_id: String,
        card_id: String,
        comment_id: String,
        #[arg(long)] json: bool,
    },
}

pub fn run(storage: &mut Storage, cmd: CommentCommand) -> anyhow::Result<()> {
    match cmd {
        CommentCommand::Add { board_id, card_id, text, json } => {
            let mut doc = storage.load_board(&board_id)?;
            let identity = storage.load_identity()?;
            let comment = add_comment(&mut doc, &card_id, &text, &identity.public_key)?;
            storage.save_board(&board_id, &doc)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&comment)?);
            } else {
                println!("Added comment {}", comment.id);
            }
        }
        CommentCommand::List { board_id, card_id, json } => {
            let doc = storage.load_board(&board_id)?;
            let comments = list_comments(&doc, &card_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&comments)?);
            } else {
                for c in &comments {
                    println!("[{}] {}: {}", c.created_at, c.author_alias, c.text);
                }
            }
        }
        CommentCommand::Delete { board_id, card_id, comment_id, json } => {
            let mut doc = storage.load_board(&board_id)?;
            let identity = storage.load_identity()?;
            delete_comment(&mut doc, &card_id, &comment_id, &identity.public_key)?;
            storage.save_board(&board_id, &doc)?;
            if json {
                println!("{{\"deleted\": \"{comment_id}\"}}");
            } else {
                println!("Deleted comment {comment_id}");
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Register in main.rs**

In `crates/kanban-cli/src/main.rs`, add `Comment(CommentArgs)` variant to the card subcommand and route to `commands::comment::run`.

- [ ] **Step 5: Run tests**

```bash
cargo test -p kanban-cli cli_comment
```
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/kanban-cli/src/commands/comment.rs crates/kanban-cli/src/main.rs
git commit -m "feat(cli): add card comment add/list/delete commands"
```

---

### Task 7: Checklist CLI commands

**Files:**
- Create: `crates/kanban-cli/src/commands/checklist.rs`
- Modify: `crates/kanban-cli/src/main.rs`

- [ ] **Step 1: Write the integration test**

```rust
#[test]
fn checklist_add_and_item_check() {
    // Setup: init, create board/column/card (same pattern as Task 6)
    // ...

    // Add checklist
    let cl_out = run_cli(&tmp, &["checklist", "add", board_id, card_id, "QA Steps", "--json"]);
    let cl: serde_json::Value = serde_json::from_slice(&cl_out.stdout).unwrap();
    let cl_id = cl["id"].as_str().unwrap();

    // Add item
    let item_out = run_cli(&tmp, &["checklist", "item", "add", board_id, card_id, cl_id, "Write tests", "--json"]);
    let item: serde_json::Value = serde_json::from_slice(&item_out.stdout).unwrap();
    let item_id = item["id"].as_str().unwrap();

    // Check item
    let check_out = run_cli(&tmp, &["checklist", "item", "check", board_id, card_id, cl_id, item_id, "--json"]);
    assert!(check_out.status.success());

    // Verify item is checked
    let cl_list = run_cli(&tmp, &["card", "view", board_id, card_id, "--json"]);
    let card: serde_json::Value = serde_json::from_slice(&cl_list.stdout).unwrap();
    assert_eq!(card["checklists"][0]["items"][0]["checked"], true);
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-cli checklist_add_and_item
```
Expected: FAIL.

- [ ] **Step 3: Implement checklist commands**

Create `crates/kanban-cli/src/commands/checklist.rs` following the same pattern as `comment.rs`. Commands map to:
- `checklist add <board> <card> <title>` → `kanban_core::checklist::add_checklist`
- `checklist rename <board> <card> <cl_id> <title>` → `kanban_core::checklist::rename_checklist`
- `checklist delete <board> <card> <cl_id>` → `kanban_core::checklist::delete_checklist`
- `checklist item add <board> <card> <cl_id> <text>` → `kanban_core::checklist::add_item`
- `checklist item check <board> <card> <cl_id> <item_id>` → `kanban_core::checklist::check_item`
- `checklist item uncheck <board> <card> <cl_id> <item_id>` → `kanban_core::checklist::uncheck_item`
- `checklist item delete <board> <card> <cl_id> <item_id>` → `kanban_core::checklist::delete_item`

All commands accept `--json`. JSON output for mutations returns `{ "id": "...", "board_id": "...", "hlc": "..." }`.

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-cli checklist
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-cli/src/commands/checklist.rs crates/kanban-cli/src/main.rs
git commit -m "feat(cli): add checklist and checklist-item CRUD commands"
```

---

### Task 8: `copy_card` Tauri command

**Files:**
- Modify: `crates/kanban-tauri/src-tauri/src/commands/card.rs`

- [ ] **Step 1: Write the test**

In `crates/kanban-tauri/src-tauri/src/commands/card.rs`, add a unit test:
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn copy_card_command_returns_new_card() {
        // Use a test AppState with an in-memory board
        let state = build_test_state();
        let result = copy_card_handler(
            state,
            CopyCardArgs {
                board_id: "board1".into(),
                card_id: "card-uuid-1".into(),
                to_column: None,
            },
        );
        assert!(result.is_ok());
        let card = result.unwrap();
        assert_ne!(card.id, "card-uuid-1");
        assert!(card.title.starts_with("Copy of"));
    }
}
```

- [ ] **Step 2: Implement the Tauri command**

```rust
#[derive(serde::Deserialize)]
pub struct CopyCardArgs {
    pub board_id: String,
    pub card_id: String,
    pub to_column: Option<String>,
}

#[tauri::command]
pub async fn copy_card(
    state: tauri::State<'_, AppState>,
    args: CopyCardArgs,
) -> Result<CardSummary, String> {
    let mut state = state.lock().await;
    let identity = state.identity.clone();
    let members = state.get_board_members(&args.board_id).map_err(|e| e.to_string())?;
    let mut doc = state.load_board(&args.board_id).map_err(|e| e.to_string())?;

    let col_id = match &args.to_column {
        Some(col) => state.resolve_column(&doc, col).map_err(|e| e.to_string())?,
        None => state.get_card_column(&doc, &args.card_id).map_err(|e| e.to_string())?,
    };

    let card = kanban_core::card::copy_card(
        &mut doc,
        &args.card_id,
        col_id,
        &identity.public_key,
        &members,
    )
    .map_err(|e| e.to_string())?;

    state.save_board(&args.board_id, &doc).map_err(|e| e.to_string())?;
    state.broadcast_changes(&args.board_id).await.map_err(|e| e.to_string())?;

    Ok(CardSummary::from(card))
}
```

- [ ] **Step 3: Register command in Tauri builder**

In `crates/kanban-tauri/src-tauri/src/main.rs`, add `copy_card` to the `.invoke_handler(tauri::generate_handler![...])` list.

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-tauri copy_card_command
```
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-tauri/src-tauri/src/commands/card.rs crates/kanban-tauri/src-tauri/src/main.rs
git commit -m "feat(tauri): add copy_card Tauri command"
```

---

### Task 9: Wire `sync_card_number_index` after every Automerge merge

**Files:**
- Modify: `crates/kanban-storage/src/lib.rs`

This ensures the index stays current whether changes come from local ops or peer gossip.

- [ ] **Step 1: Write the test**

```rust
#[test]
fn index_updated_after_peer_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let mut storage = Storage::open(tmp.path()).unwrap();
    let actor_pk = vec![1u8; 32];
    let members = vec![actor_pk.clone()];

    // Simulate receiving a peer's Automerge change containing a new card with a number
    let peer_doc_bytes = build_peer_doc_with_card(&actor_pk, &members, "a7f3-1");
    storage.merge_board_changes("board1", &peer_doc_bytes).unwrap();

    // Index should be populated
    let uuid = storage.resolve_card_ref("board1", "a7f3-1").unwrap();
    assert!(!uuid.is_empty());
}
```

- [ ] **Step 2: Modify `merge_board_changes` (or equivalent function) to call sync**

In `crates/kanban-storage/src/lib.rs`, after every `AutoCommit::merge` or `AutoCommit::apply_changes` call:
```rust
// After merging:
let changed_cards = extract_changed_card_numbers(&doc)?; // scan doc for cards with a "number" field
kanban_storage::card_number::sync_card_number_index(&self.conn, board_id, &changed_cards)?;
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p kanban-storage index_updated_after_peer
```
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-storage/src/lib.rs
git commit -m "feat(storage): sync card_number_index after every Automerge merge"
```

---

### Task 10: End-to-end smoke test

- [ ] **Step 1: Write the smoke test**

In `tests/integration/phase1_smoke_test.rs`:
```rust
/// Full flow: create card → verify number → copy card → verify copy number → add comment → add checklist
#[test]
fn phase1_full_flow() {
    let tmp = tempfile::tempdir().unwrap();
    // ... use CLI binary to drive the full flow ...
    // Verify card numbers appear in `card view --json` output
    // Verify copy has "Copy of" title and incremented number
    // Verify comment appears in `comment list --json`
    // Verify checklist item can be checked
}
```

- [ ] **Step 2: Run**

```bash
cargo test --test phase1_smoke_test
```
Expected: pass.

- [ ] **Step 3: Final commit**

```bash
git add tests/integration/phase1_smoke_test.rs
git commit -m "test(integration): phase1 smoke test for card numbers, copy, comments, checklists"
```
