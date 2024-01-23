# Phase 0: Cargo Workspace Scaffold

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the Cargo workspace with all 6 crates so `cargo build --workspace` passes and subsequent feature plans have real types and functions to build on.

**Architecture:** Rust 2021 Cargo workspace. Four library crates (`kanban-core`, `kanban-crypto`, `kanban-net`, `kanban-storage`) share domain logic. Two binary crates (`kanban-cli`, `kanban-tauri`) import them directly — no CLI shelling-out. Core types are defined in `kanban-core`; storage and crypto are separate concerns.

**Tech Stack:** Rust 1.75+, `automerge = "0.5"`, `rusqlite = "0.31"`, `ed25519-dalek = "2"`, `iroh = "0.26"`, `clap = "4"`, `thiserror = "1"`, `anyhow = "1"`, `tracing = "0.1"`, `serde`, `uuid`, `hex`, `base32`, `ciborium`

**Working directory for all commands:** `/Users/morteza/Desktop/monoes/monotask/.worktrees/phase1-features`

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| Create | `Cargo.toml` | Workspace root — members list |
| Create | `crates/kanban-core/Cargo.toml` | Core library deps |
| Create | `crates/kanban-core/src/lib.rs` | Re-exports, Error type |
| Create | `crates/kanban-core/src/board.rs` | Board struct, create/rename/delete |
| Create | `crates/kanban-core/src/column.rs` | Column struct, create/rename/delete/move |
| Create | `crates/kanban-core/src/card.rs` | Card struct, create/edit/move/delete/archive |
| Create | `crates/kanban-core/src/comment.rs` | Comment add/list/delete |
| Create | `crates/kanban-core/src/checklist.rs` | Checklist + item CRUD |
| Create | `crates/kanban-core/src/clock.rs` | HLC timestamp |
| Create | `crates/kanban-crypto/Cargo.toml` | Crypto deps |
| Create | `crates/kanban-crypto/src/lib.rs` | Identity, keypair gen, sign/verify |
| Create | `crates/kanban-net/Cargo.toml` | Net deps (stub) |
| Create | `crates/kanban-net/src/lib.rs` | Stub — Error type only |
| Create | `crates/kanban-storage/Cargo.toml` | Storage deps |
| Create | `crates/kanban-storage/src/lib.rs` | Storage struct, open/close |
| Create | `crates/kanban-storage/src/schema.rs` | SQLite migrations |
| Create | `crates/kanban-storage/src/board.rs` | save_board, load_board, list_boards |
| Create | `crates/kanban-cli/Cargo.toml` | CLI deps |
| Create | `crates/kanban-cli/src/main.rs` | clap root with board/column/card subcommands (stubs) |
| Create | `crates/kanban-tauri/src-tauri/Cargo.toml` | Tauri deps |
| Create | `crates/kanban-tauri/src-tauri/src/main.rs` | Minimal Tauri main (compiles, no commands yet) |
| Create | `crates/kanban-tauri/src-tauri/tauri.conf.json` | Tauri config |

---

### Task 1: Workspace root and `kanban-core` types

**Files:**
- Create: `Cargo.toml` (workspace)
- Create: `crates/kanban-core/Cargo.toml`
- Create: `crates/kanban-core/src/lib.rs`
- Create: `crates/kanban-core/src/board.rs`
- Create: `crates/kanban-core/src/column.rs`
- Create: `crates/kanban-core/src/card.rs`
- Create: `crates/kanban-core/src/comment.rs`
- Create: `crates/kanban-core/src/checklist.rs`
- Create: `crates/kanban-core/src/clock.rs`

- [ ] **Step 1: Create workspace Cargo.toml**

```toml
# Cargo.toml (workspace root)
[workspace]
members = [
    "crates/kanban-core",
    "crates/kanban-crypto",
    "crates/kanban-net",
    "crates/kanban-storage",
    "crates/kanban-cli",
    "crates/kanban-tauri/src-tauri",
]
resolver = "2"

[workspace.dependencies]
automerge   = "0.5"
rusqlite    = { version = "0.31", features = ["bundled"] }
ed25519-dalek = { version = "2", features = ["rand_core"] }
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
ciborium    = "0.2"
thiserror   = "1"
anyhow      = "1"
tracing     = "0.1"
uuid        = { version = "1", features = ["v4", "v7"] }
hex         = "0.4"
base32      = "0.4"
regex       = "1"
chrono      = { version = "0.4", features = ["serde"] }
rand        = "0.8"
tokio       = { version = "1", features = ["full"] }
clap        = { version = "4", features = ["derive"] }
tabled      = "0.15"
colored     = "2"
tempfile    = "3"
dirs        = "5"

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 2: Create `kanban-core/Cargo.toml`**

```toml
[package]
name = "kanban-core"
version = "0.1.0"
edition = "2021"

[dependencies]
automerge  = { workspace = true }
serde      = { workspace = true }
serde_json = { workspace = true }
thiserror  = { workspace = true }
tracing    = { workspace = true }
uuid       = { workspace = true }
hex        = { workspace = true }
base32     = { workspace = true }
regex      = { workspace = true }
chrono     = { workspace = true }
rand       = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 3: Write failing test for Card creation**

Create `crates/kanban-core/src/card.rs` — write the test first:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use automerge::AutoCommit;

    #[test]
    fn create_card_stores_title() {
        let mut doc = AutoCommit::new();
        init_doc(&mut doc);
        let col_id = create_column(&mut doc, "To Do").unwrap();
        let card = create_card(&mut doc, &col_id, "My Task").unwrap();
        assert_eq!(card.title, "My Task");
        assert!(!card.id.is_empty());
    }
}
```

Run: `cargo test -p kanban-core create_card_stores_title`
Expected: compile error (types not defined yet)

- [ ] **Step 4: Create `kanban-core/src/lib.rs`**

```rust
pub mod board;
pub mod card;
pub mod checklist;
pub mod clock;
pub mod column;
pub mod comment;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("automerge error: {0}")]
    Automerge(#[from] automerge::AutomergeError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid document: {0}")]
    InvalidDocument(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

// Shared helpers used across modules

use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, ROOT, transaction::Transactable};

/// Initialize a new Automerge document with the p2p-kanban root structure.
pub fn init_doc(doc: &mut AutoCommit) {
    // Only initialise once
    if doc.get(ROOT, "columns").ok().flatten().is_some() {
        return;
    }
    let _ = doc.put_object(ROOT, "columns", ObjType::List);
    let _ = doc.put_object(ROOT, "cards", ObjType::Map);
    let _ = doc.put_object(ROOT, "members", ObjType::Map);
    let _ = doc.put_object(ROOT, "actor_card_seq", ObjType::Map);
    let _ = doc.put_object(ROOT, "label_definitions", ObjType::Map);
}

/// Return the `cards` map ObjId (read-write).
pub fn get_cards_map(doc: &mut AutoCommit) -> Result<ObjId> {
    match doc.get(ROOT, "cards")? {
        Some((_, id)) => Ok(id),
        None => Err(Error::InvalidDocument("missing cards map".into())),
    }
}

/// Return the `cards` map ObjId (read-only).
pub fn get_cards_map_readonly(doc: &AutoCommit) -> Result<ObjId> {
    match doc.get(ROOT, "cards")? {
        Some((_, id)) => Ok(id),
        None => Err(Error::InvalidDocument("missing cards map".into())),
    }
}

/// Return the `columns` list ObjId.
pub fn get_columns_list(doc: &mut AutoCommit) -> Result<ObjId> {
    match doc.get(ROOT, "columns")? {
        Some((_, id)) => Ok(id),
        None => Err(Error::InvalidDocument("missing columns list".into())),
    }
}

pub fn get_string(doc: &AutoCommit, obj: &ObjId, key: &str) -> Result<Option<String>> {
    match doc.get(obj, key)? {
        Some((automerge::Value::Scalar(s), _)) => {
            if let automerge::ScalarValue::Str(text) = s.as_ref() {
                Ok(Some(text.to_string()))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}
```

- [ ] **Step 5: Create `crates/kanban-core/src/clock.rs`**

```rust
use chrono::Utc;
use std::sync::atomic::{AtomicU64, Ordering};

static LOGICAL: AtomicU64 = AtomicU64::new(0);

/// Returns an HLC timestamp string: "<wall_ms>-<logical>".
pub fn now() -> String {
    let wall = Utc::now().timestamp_millis() as u64;
    let logical = LOGICAL.fetch_add(1, Ordering::SeqCst);
    format!("{wall:016x}-{logical:08x}")
}
```

- [ ] **Step 6: Create `crates/kanban-core/src/column.rs`**

```rust
use automerge::{AutoCommit, ObjType, ReadDoc, transaction::Transactable};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub id: String,
    pub title: String,
}

pub fn create_column(doc: &mut AutoCommit, title: &str) -> Result<String> {
    let col_id = uuid::Uuid::new_v4().to_string();
    let cols = crate::get_columns_list(doc)?;
    let idx = doc.length(&cols);
    let col_obj = doc.insert_object(&cols, idx, ObjType::Map)?;
    doc.put(&col_obj, "id", col_id.as_str())?;
    doc.put(&col_obj, "title", title)?;
    doc.put_object(&col_obj, "card_ids", ObjType::List)?;
    Ok(col_id)
}

pub fn rename_column(doc: &mut AutoCommit, col_obj: &automerge::ObjId, new_title: &str) -> Result<()> {
    doc.put(col_obj, "title", new_title)?;
    Ok(())
}

pub fn find_column_obj(doc: &AutoCommit, col_id: &str) -> Result<Option<automerge::ObjId>> {
    let cols = match doc.get(automerge::ROOT, "columns")? {
        Some((_, id)) => id,
        None => return Ok(None),
    };
    for i in 0..doc.length(&cols) {
        if let Some((_, obj)) = doc.get(&cols, i)? {
            if let Ok(Some(id)) = crate::get_string(doc, &obj, "id") {
                if id == col_id {
                    return Ok(Some(obj));
                }
            }
        }
    }
    Ok(None)
}

pub fn get_card_ids_list(doc: &AutoCommit, col_obj: &automerge::ObjId) -> Result<automerge::ObjId> {
    match doc.get(col_obj, "card_ids")? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::InvalidDocument("column missing card_ids".into())),
    }
}

pub fn append_card_to_column(doc: &mut AutoCommit, col_id: &str, card_id: &str) -> Result<()> {
    let col_obj = find_column_obj(doc, col_id)?
        .ok_or_else(|| crate::Error::NotFound(format!("column {col_id}")))?;
    let card_ids = get_card_ids_list(doc, &col_obj)?;
    let idx = doc.length(&card_ids);
    doc.insert(&card_ids, idx, card_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::AutoCommit;

    #[test]
    fn create_column_stores_title() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc);
        let id = create_column(&mut doc, "Backlog").unwrap();
        assert!(!id.is_empty());
        let obj = find_column_obj(&doc, &id).unwrap().unwrap();
        let title = crate::get_string(&doc, &obj, "title").unwrap();
        assert_eq!(title, Some("Backlog".to_string()));
    }
}
```

- [ ] **Step 7: Create `crates/kanban-core/src/card.rs`**

```rust
use automerge::{AutoCommit, ObjType, ReadDoc, ScalarValue, transaction::Transactable};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub description: String,
    pub assignees: Vec<String>,
    pub labels: Vec<String>,
    pub due_date: Option<String>,
    pub archived: bool,
    pub deleted: bool,
    pub copied_from: Option<String>,
    pub created_by: String,
    pub created_at: String,
}

pub fn create_card(doc: &mut AutoCommit, col_id: &str, title: &str) -> Result<Card> {
    let card_id = uuid::Uuid::new_v4().to_string();
    let hlc = crate::clock::now();
    let cards_map = crate::get_cards_map(doc)?;
    let card_obj = doc.put_object(&cards_map, &card_id, ObjType::Map)?;
    doc.put(&card_obj, "id", card_id.as_str())?;
    doc.put(&card_obj, "title", title)?;
    doc.put(&card_obj, "description", "")?;
    doc.put(&card_obj, "created_at", hlc.as_str())?;
    doc.put(&card_obj, "deleted", false)?;
    doc.put(&card_obj, "archived", false)?;
    doc.put_object(&card_obj, "assignees", ObjType::List)?;
    doc.put_object(&card_obj, "labels", ObjType::List)?;
    doc.put_object(&card_obj, "comments", ObjType::List)?;
    doc.put_object(&card_obj, "checklists", ObjType::List)?;
    doc.put_object(&card_obj, "related", ObjType::Map)?;
    crate::column::append_card_to_column(doc, col_id, &card_id)?;
    Ok(Card {
        id: card_id,
        title: title.to_string(),
        created_at: hlc,
        ..Default::default()
    })
}

pub fn get_card_obj(doc: &AutoCommit, card_id: &str) -> Result<automerge::ObjId> {
    let cards_map = crate::get_cards_map_readonly(doc)?;
    match doc.get(&cards_map, card_id)? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::NotFound(format!("card {card_id}"))),
    }
}

pub fn is_tombstoned(doc: &AutoCommit, card_id: &str) -> Result<bool> {
    let cards_map = crate::get_cards_map_readonly(doc)?;
    match doc.get(&cards_map, card_id)? {
        None => Ok(true), // absent = effectively tombstoned
        Some((_, obj)) => {
            match doc.get(&obj, "deleted")? {
                Some((automerge::Value::Scalar(s), _)) => {
                    if let ScalarValue::Boolean(b) = s.as_ref() {
                        Ok(*b)
                    } else {
                        Ok(false)
                    }
                }
                _ => Ok(false),
            }
        }
    }
}

pub fn rename_card(doc: &mut AutoCommit, card_id: &str, new_title: &str) -> Result<()> {
    let cards_map = crate::get_cards_map(doc)?;
    let card_obj = match doc.get(&cards_map, card_id)? {
        Some((_, id)) => id,
        None => return Err(crate::Error::NotFound(card_id.into())),
    };
    doc.put(&card_obj, "title", new_title)?;
    Ok(())
}

pub fn delete_card(doc: &mut AutoCommit, card_id: &str) -> Result<()> {
    let cards_map = crate::get_cards_map(doc)?;
    let card_obj = match doc.get(&cards_map, card_id)? {
        Some((_, id)) => id,
        None => return Err(crate::Error::NotFound(card_id.into())),
    };
    doc.put(&card_obj, "deleted", true)?;
    doc.put(&card_obj, "deleted_at", crate::clock::now().as_str())?;
    Ok(())
}

pub fn get_card_display_name(doc: &AutoCommit, card_id: &str) -> Result<Option<String>> {
    let cards_map = crate::get_cards_map_readonly(doc)?;
    match doc.get(&cards_map, card_id)? {
        None => Ok(None),
        Some((_, obj)) => {
            let title = crate::get_string(doc, &obj, "title")?;
            let number = crate::get_string(doc, &obj, "number")?;
            match (number, title) {
                (Some(n), Some(t)) => Ok(Some(format!("#{n} — {t}"))),
                (None, Some(t)) => Ok(Some(t)),
                _ => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::AutoCommit;

    #[test]
    fn create_card_stores_title() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc);
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        let card = create_card(&mut doc, &col_id, "My Task").unwrap();
        assert_eq!(card.title, "My Task");
        assert!(!card.id.is_empty());
    }

    #[test]
    fn delete_card_sets_tombstone() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc);
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        let card = create_card(&mut doc, &col_id, "Task").unwrap();
        delete_card(&mut doc, &card.id).unwrap();
        assert!(is_tombstoned(&doc, &card.id).unwrap());
    }
}
```

- [ ] **Step 8: Create `crates/kanban-core/src/comment.rs`**

```rust
use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, transaction::Transactable};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub text: String,
    pub created_at: String,
    pub deleted: bool,
}

pub fn get_comments_list(doc: &AutoCommit, card_obj: &ObjId) -> Result<ObjId> {
    match doc.get(card_obj, "comments")? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::InvalidDocument("card missing comments list".into())),
    }
}

pub fn add_comment(doc: &mut AutoCommit, card_id: &str, text: &str, author_key: &str) -> Result<Comment> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let comments = get_comments_list(doc, &card_obj)?;
    let idx = doc.length(&comments);
    let comment_id = uuid::Uuid::new_v4().to_string();
    let hlc = crate::clock::now();
    let c_obj = doc.insert_object(&comments, idx, ObjType::Map)?;
    doc.put(&c_obj, "id", comment_id.as_str())?;
    doc.put(&c_obj, "author", author_key)?;
    doc.put(&c_obj, "text", text)?;
    doc.put(&c_obj, "created_at", hlc.as_str())?;
    doc.put(&c_obj, "deleted", false)?;
    Ok(Comment { id: comment_id, author: author_key.into(), text: text.into(), created_at: hlc, deleted: false })
}

pub fn delete_comment(doc: &mut AutoCommit, card_id: &str, comment_id: &str) -> Result<()> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let comments = get_comments_list(doc, &card_obj)?;
    for i in 0..doc.length(&comments) {
        if let Some((_, c_obj)) = doc.get(&comments, i)? {
            if let Ok(Some(id)) = crate::get_string(doc, &c_obj, "id") {
                if id == comment_id {
                    doc.put(&c_obj, "deleted", true)?;
                    return Ok(());
                }
            }
        }
    }
    Err(crate::Error::NotFound(comment_id.into()))
}

pub fn list_comments(doc: &AutoCommit, card_id: &str) -> Result<Vec<Comment>> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let comments = get_comments_list(doc, &card_obj)?;
    let mut result = Vec::new();
    for i in 0..doc.length(&comments) {
        if let Some((_, c_obj)) = doc.get(&comments, i)? {
            let deleted = matches!(
                doc.get(&c_obj, "deleted")?,
                Some((automerge::Value::Scalar(s), _)) if matches!(s.as_ref(), automerge::ScalarValue::Boolean(true))
            );
            if !deleted {
                result.push(Comment {
                    id: crate::get_string(doc, &c_obj, "id")?.unwrap_or_default(),
                    author: crate::get_string(doc, &c_obj, "author")?.unwrap_or_default(),
                    text: crate::get_string(doc, &c_obj, "text")?.unwrap_or_default(),
                    created_at: crate::get_string(doc, &c_obj, "created_at")?.unwrap_or_default(),
                    deleted: false,
                });
            }
        }
    }
    Ok(result)
}
```

- [ ] **Step 9: Create `crates/kanban-core/src/checklist.rs`**

```rust
use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, transaction::Transactable};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: String,
    pub text: String,
    pub checked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checklist {
    pub id: String,
    pub title: String,
    pub items: Vec<ChecklistItem>,
}

fn get_checklists_list(doc: &AutoCommit, card_obj: &ObjId) -> Result<ObjId> {
    match doc.get(card_obj, "checklists")? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::InvalidDocument("card missing checklists list".into())),
    }
}

pub fn add_checklist(doc: &mut AutoCommit, card_id: &str, title: &str) -> Result<Checklist> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let cls = get_checklists_list(doc, &card_obj)?;
    let idx = doc.length(&cls);
    let cl_id = uuid::Uuid::new_v4().to_string();
    let cl_obj = doc.insert_object(&cls, idx, ObjType::Map)?;
    doc.put(&cl_obj, "id", cl_id.as_str())?;
    doc.put(&cl_obj, "title", title)?;
    doc.put_object(&cl_obj, "items", ObjType::List)?;
    Ok(Checklist { id: cl_id, title: title.into(), items: vec![] })
}

pub fn add_checklist_item(doc: &mut AutoCommit, card_id: &str, cl_id: &str, text: &str) -> Result<ChecklistItem> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let cls = get_checklists_list(doc, &card_obj)?;
    for i in 0..doc.length(&cls) {
        if let Some((_, cl_obj)) = doc.get(&cls, i)? {
            if crate::get_string(doc, &cl_obj, "id")?.as_deref() == Some(cl_id) {
                let items = match doc.get(&cl_obj, "items")? {
                    Some((_, id)) => id,
                    None => doc.put_object(&cl_obj, "items", ObjType::List)?,
                };
                let item_id = uuid::Uuid::new_v4().to_string();
                let idx = doc.length(&items);
                let item_obj = doc.insert_object(&items, idx, ObjType::Map)?;
                doc.put(&item_obj, "id", item_id.as_str())?;
                doc.put(&item_obj, "text", text)?;
                doc.put(&item_obj, "checked", false)?;
                return Ok(ChecklistItem { id: item_id, text: text.into(), checked: false });
            }
        }
    }
    Err(crate::Error::NotFound(cl_id.into()))
}

pub fn set_item_checked(doc: &mut AutoCommit, card_id: &str, cl_id: &str, item_id: &str, checked: bool) -> Result<()> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let cls = get_checklists_list(doc, &card_obj)?;
    for i in 0..doc.length(&cls) {
        if let Some((_, cl_obj)) = doc.get(&cls, i)? {
            if crate::get_string(doc, &cl_obj, "id")?.as_deref() == Some(cl_id) {
                let items = match doc.get(&cl_obj, "items")? {
                    Some((_, id)) => id,
                    None => return Err(crate::Error::NotFound(cl_id.into())),
                };
                for j in 0..doc.length(&items) {
                    if let Some((_, item_obj)) = doc.get(&items, j)? {
                        if crate::get_string(doc, &item_obj, "id")?.as_deref() == Some(item_id) {
                            doc.put(&item_obj, "checked", checked)?;
                            return Ok(());
                        }
                    }
                }
                return Err(crate::Error::NotFound(item_id.into()));
            }
        }
    }
    Err(crate::Error::NotFound(cl_id.into()))
}
```

- [ ] **Step 10: Create `crates/kanban-core/src/board.rs`**

```rust
use automerge::{AutoCommit, ObjType, ReadDoc, transaction::Transactable, ROOT};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub id: String,
    pub title: String,
    pub created_at: String,
}

pub fn create_board(title: &str, created_by: &str) -> (AutoCommit, Board) {
    let mut doc = AutoCommit::new();
    crate::init_doc(&mut doc);
    let id = uuid::Uuid::new_v4().to_string();
    let hlc = crate::clock::now();
    let _ = doc.put(ROOT, "id", id.as_str());
    let _ = doc.put(ROOT, "title", title);
    let _ = doc.put(ROOT, "created_at", hlc.as_str());
    let _ = doc.put(ROOT, "created_by", created_by);
    let board = Board { id, title: title.into(), created_at: hlc };
    (doc, board)
}

pub fn get_board_title(doc: &AutoCommit) -> Result<String> {
    crate::get_string(doc, &ROOT, "title")?
        .ok_or_else(|| crate::Error::InvalidDocument("board missing title".into()))
}
```

- [ ] **Step 11: Run all core tests**

```bash
cd /Users/morteza/Desktop/monoes/monotask/.worktrees/phase1-features
cargo test -p kanban-core
```
Expected: all tests pass (create_card_stores_title, delete_card_sets_tombstone, create_column_stores_title).

- [ ] **Step 12: Commit**

```bash
git add crates/kanban-core/
git commit -m "feat(core): kanban-core domain types — Board, Column, Card, Comment, Checklist"
```

---

### Task 2: `kanban-crypto` — Ed25519 identity

**Files:**
- Create: `crates/kanban-crypto/Cargo.toml`
- Create: `crates/kanban-crypto/src/lib.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "kanban-crypto"
version = "0.1.0"
edition = "2021"

[dependencies]
ed25519-dalek = { workspace = true }
thiserror     = { workspace = true }
tracing       = { workspace = true }
hex           = { workspace = true }
base32        = { workspace = true }
rand          = { workspace = true }
serde         = { workspace = true }
ciborium      = { workspace = true }
```

- [ ] **Step 2: Write failing test**

```rust
// In kanban-crypto/src/lib.rs tests:
#[test]
fn generate_identity_has_stable_pubkey() {
    let id = Identity::generate();
    let pk1 = id.public_key_hex();
    let pk2 = id.public_key_hex();
    assert_eq!(pk1, pk2);
    assert_eq!(pk1.len(), 64); // 32 bytes hex-encoded
}
```

Run: `cargo test -p kanban-crypto`
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
use ed25519_dalek::{SigningKey, Signature, Signer, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("invalid key")]
    InvalidKey,
    #[error("signature verification failed")]
    VerifyFailed,
}

pub struct Identity {
    signing_key: SigningKey,
}

impl Identity {
    pub fn generate() -> Self {
        Self { signing_key: SigningKey::generate(&mut OsRng) }
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key_bytes())
    }

    /// base32-encoded public key with `pk_` prefix (human-readable node ID)
    pub fn node_id(&self) -> String {
        let encoded = base32::encode(
            base32::Alphabet::RFC4648 { padding: false },
            &self.public_key_bytes(),
        ).to_lowercase();
        format!("pk_{encoded}")
    }

    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        let sig: Signature = self.signing_key.sign(msg);
        sig.to_bytes().to_vec()
    }

    pub fn verify(pubkey_bytes: &[u8; 32], msg: &[u8], sig_bytes: &[u8]) -> Result<(), CryptoError> {
        let vk = VerifyingKey::from_bytes(pubkey_bytes).map_err(|_| CryptoError::InvalidKey)?;
        let sig = Signature::from_bytes(sig_bytes.try_into().map_err(|_| CryptoError::InvalidKey)?);
        vk.verify(msg, &sig).map_err(|_| CryptoError::VerifyFailed)
    }

    /// Export signing key bytes (for secure storage — caller is responsible for encryption)
    pub fn to_secret_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Restore from stored key bytes
    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Self {
        Self { signing_key: SigningKey::from_bytes(bytes) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_identity_has_stable_pubkey() {
        let id = Identity::generate();
        let pk1 = id.public_key_hex();
        let pk2 = id.public_key_hex();
        assert_eq!(pk1, pk2);
        assert_eq!(pk1.len(), 64);
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let id = Identity::generate();
        let msg = b"hello world";
        let sig = id.sign(msg);
        let pk = id.public_key_bytes();
        Identity::verify(&pk, msg, &sig).unwrap();
    }

    #[test]
    fn verify_rejects_wrong_message() {
        let id = Identity::generate();
        let sig = id.sign(b"correct");
        let pk = id.public_key_bytes();
        assert!(Identity::verify(&pk, b"wrong", &sig).is_err());
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-crypto
```
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-crypto/
git commit -m "feat(crypto): Ed25519 Identity with sign/verify and node_id"
```

---

### Task 3: `kanban-storage` — SQLite persistence

**Files:**
- Create: `crates/kanban-storage/Cargo.toml`
- Create: `crates/kanban-storage/src/lib.rs`
- Create: `crates/kanban-storage/src/schema.rs`
- Create: `crates/kanban-storage/src/board.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "kanban-storage"
version = "0.1.0"
edition = "2021"

[dependencies]
automerge    = { workspace = true }
rusqlite     = { workspace = true }
kanban-core  = { path = "../kanban-core" }
thiserror    = { workspace = true }
tracing      = { workspace = true }
serde        = { workspace = true }
serde_json   = { workspace = true }
chrono       = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 2: Write failing test**

```rust
#[test]
fn save_and_load_board_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let mut storage = Storage::open(tmp.path()).unwrap();
    let mut doc = automerge::AutoCommit::new();
    kanban_core::init_doc(&mut doc);
    storage.save_board("board1", &mut doc).unwrap();
    let loaded = storage.load_board("board1").unwrap();
    assert!(!automerge::ReadDoc::get(&loaded, automerge::ROOT, "columns").unwrap().is_none());
}
```

- [ ] **Step 3: Create `schema.rs`**

```rust
use rusqlite::{Connection, Result};

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch("
        PRAGMA journal_mode=WAL;
        PRAGMA foreign_keys=ON;

        CREATE TABLE IF NOT EXISTS boards (
            board_id      TEXT PRIMARY KEY,
            automerge_doc BLOB NOT NULL,
            last_modified INTEGER NOT NULL DEFAULT (unixepoch()),
            last_heads    TEXT
        );

        CREATE TABLE IF NOT EXISTS meta (
            board_id TEXT NOT NULL,
            key      TEXT NOT NULL,
            value    TEXT NOT NULL,
            PRIMARY KEY (board_id, key)
        );

        CREATE TABLE IF NOT EXISTS card_number_index (
            board_id TEXT NOT NULL,
            card_id  TEXT NOT NULL,
            number   TEXT NOT NULL,
            PRIMARY KEY (board_id, card_id)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_card_number_lookup
            ON card_number_index (board_id, number);

        CREATE TABLE IF NOT EXISTS mention_index (
            board_id     TEXT NOT NULL,
            card_id      TEXT NOT NULL,
            mentioned    TEXT NOT NULL,
            mentioned_by TEXT NOT NULL,
            context      TEXT NOT NULL,
            hlc          TEXT NOT NULL,
            seen         INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (board_id, card_id, mentioned, context)
        );
        CREATE INDEX IF NOT EXISTS idx_mention_unseen
            ON mention_index (mentioned, seen, hlc DESC);

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
    ")?;
    Ok(())
}
```

- [ ] **Step 4: Create `board.rs`**

```rust
use automerge::AutoCommit;
use rusqlite::params;
use crate::{Storage, StorageError};

pub fn save_board(conn: &rusqlite::Connection, board_id: &str, doc: &mut AutoCommit) -> Result<(), StorageError> {
    let bytes = doc.save();
    conn.execute(
        "INSERT INTO boards (board_id, automerge_doc, last_modified)
         VALUES (?1, ?2, unixepoch())
         ON CONFLICT(board_id) DO UPDATE SET
             automerge_doc = excluded.automerge_doc,
             last_modified = excluded.last_modified",
        params![board_id, bytes],
    )?;
    Ok(())
}

pub fn load_board(conn: &rusqlite::Connection, board_id: &str) -> Result<AutoCommit, StorageError> {
    let bytes: Vec<u8> = conn.query_row(
        "SELECT automerge_doc FROM boards WHERE board_id = ?1",
        params![board_id],
        |r| r.get(0),
    ).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(board_id.into()),
        other => StorageError::Sqlite(other),
    })?;
    automerge::AutoCommit::load(&bytes)
        .map_err(|e| StorageError::Automerge(e.to_string()))
}

pub fn list_board_ids(conn: &rusqlite::Connection) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare("SELECT board_id FROM boards ORDER BY last_modified DESC")?;
    let ids = stmt.query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids)
}
```

- [ ] **Step 5: Create `lib.rs`**

```rust
pub mod board;
pub mod schema;

use rusqlite::Connection;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("automerge error: {0}")]
    Automerge(String),
}

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open(dir: &Path) -> Result<Self, StorageError> {
        std::fs::create_dir_all(dir)?;
        let db_path = dir.join("kanban.db");
        let conn = Connection::open(&db_path)?;
        schema::run_migrations(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        schema::run_migrations(&conn)?;
        Ok(Self { conn })
    }

    pub fn save_board(&mut self, board_id: &str, doc: &mut automerge::AutoCommit) -> Result<(), StorageError> {
        board::save_board(&self.conn, board_id, doc)
    }

    pub fn load_board(&self, board_id: &str) -> Result<automerge::AutoCommit, StorageError> {
        board::load_board(&self.conn, board_id)
    }

    pub fn list_board_ids(&self) -> Result<Vec<String>, StorageError> {
        board::list_board_ids(&self.conn)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn resolve_card_ref(&self, board_id: &str, card_ref: &str) -> Result<String, StorageError> {
        // Try card number pattern first
        if card_ref.contains('-') && card_ref.split('-').last()
            .map(|s| s.parse::<u64>().is_ok()).unwrap_or(false)
        {
            let result: Option<String> = self.conn.query_row(
                "SELECT card_id FROM card_number_index WHERE board_id=?1 AND number=?2",
                rusqlite::params![board_id, card_ref],
                |r| r.get(0),
            ).optional()?;
            if let Some(uuid) = result {
                return Ok(uuid);
            }
        }
        // UUID passthrough
        Ok(card_ref.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::AutoCommit;

    #[test]
    fn save_and_load_board_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let mut storage = Storage::open(tmp.path()).unwrap();
        let mut doc = AutoCommit::new();
        kanban_core::init_doc(&mut doc);
        storage.save_board("board1", &mut doc).unwrap();
        let loaded = storage.load_board("board1").unwrap();
        assert!(automerge::ReadDoc::get(&loaded, automerge::ROOT, "columns").unwrap().is_some());
    }

    #[test]
    fn list_boards_returns_saved_boards() {
        let mut storage = Storage::open_in_memory().unwrap();
        let mut doc = AutoCommit::new();
        kanban_core::init_doc(&mut doc);
        storage.save_board("board-a", &mut doc).unwrap();
        storage.save_board("board-b", &mut doc).unwrap();
        let ids = storage.list_board_ids().unwrap();
        assert_eq!(ids.len(), 2);
    }
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p kanban-storage
```
Expected: 2 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/kanban-storage/
git commit -m "feat(storage): SQLite persistence with WAL, all schema tables, save/load board"
```

---

### Task 4: `kanban-net` stub + `kanban-cli` skeleton

**Files:**
- Create: `crates/kanban-net/Cargo.toml`
- Create: `crates/kanban-net/src/lib.rs`
- Create: `crates/kanban-cli/Cargo.toml`
- Create: `crates/kanban-cli/src/main.rs`

- [ ] **Step 1: Create kanban-net stub**

`crates/kanban-net/Cargo.toml`:
```toml
[package]
name = "kanban-net"
version = "0.1.0"
edition = "2021"

[dependencies]
thiserror = { workspace = true }
tracing   = { workspace = true }
tokio     = { workspace = true }
serde     = { workspace = true }
ciborium  = { workspace = true }
```

`crates/kanban-net/src/lib.rs`:
```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error")]
    Serialization,
    #[error("unknown message type: {0:#x}")]
    UnknownMessageType(u8),
    #[error("empty message")]
    EmptyMessage,
    #[error("invalid key")]
    InvalidKey,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("handshake timeout")]
    HandshakeTimeout,
    #[error("handshake io error")]
    HandshakeIo,
    #[error("unexpected message type during handshake")]
    UnexpectedMessageType,
    #[error("incompatible peer version: {0}")]
    IncompatibleVersion(String),
}

// Placeholder — real implementation in Phase 2 of base plan
pub struct NetworkHandle;

impl NetworkHandle {
    pub fn new() -> Self { Self }
}
```

- [ ] **Step 2: Create kanban-cli**

`crates/kanban-cli/Cargo.toml`:
```toml
[package]
name = "kanban-cli"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "app-cli"
path = "src/main.rs"

[dependencies]
kanban-core    = { path = "../kanban-core" }
kanban-crypto  = { path = "../kanban-crypto" }
kanban-storage = { path = "../kanban-storage" }
clap           = { workspace = true }
anyhow         = { workspace = true }
serde          = { workspace = true }
serde_json     = { workspace = true }
tabled         = { workspace = true }
colored        = { workspace = true }
tracing        = { workspace = true }
dirs           = { workspace = true }

[dev-dependencies]
tempfile   = { workspace = true }
serde_json = { workspace = true }
```

`crates/kanban-cli/src/main.rs`:
```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "app-cli", about = "P2P Kanban CLI")]
struct Cli {
    #[arg(long, global = true, help = "Data directory")]
    data_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize identity and config
    Init,
    /// Board management
    Board {
        #[command(subcommand)]
        cmd: BoardCommands,
    },
    /// Column management
    Column {
        #[command(subcommand)]
        cmd: ColumnCommands,
    },
    /// Card management
    Card {
        #[command(subcommand)]
        cmd: CardCommands,
    },
}

#[derive(Subcommand)]
enum BoardCommands {
    Create { title: String, #[arg(long)] json: bool },
    List { #[arg(long)] json: bool },
}

#[derive(Subcommand)]
enum ColumnCommands {
    Create { board_id: String, title: String, #[arg(long)] json: bool },
}

#[derive(Subcommand)]
enum CardCommands {
    Create { board_id: String, col_id: String, title: String, #[arg(long)] json: bool },
    View { board_id: String, card_id: String, #[arg(long)] json: bool },
}

fn data_dir(cli: &Cli) -> std::path::PathBuf {
    cli.data_dir.clone().unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("p2p-kanban")
    })
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let dir = data_dir(&cli);
    let mut storage = kanban_storage::Storage::open(&dir)?;

    match cli.command {
        Commands::Init => {
            println!("Initialized p2p-kanban at {}", dir.display());
        }
        Commands::Board { cmd } => match cmd {
            BoardCommands::Create { title, json } => {
                let id = kanban_crypto::Identity::generate();
                let (mut doc, board) = kanban_core::board::create_board(&title, &id.public_key_hex());
                storage.save_board(&board.id, &mut doc)?;
                if json {
                    let deep_link = format!("kanban://board/{}", board.id);
                    println!("{}", serde_json::json!({"id": board.id, "title": board.title, "deep_link": deep_link}));
                } else {
                    println!("Created board: {} ({})", board.title, board.id);
                }
            }
            BoardCommands::List { json } => {
                let ids = storage.list_board_ids()?;
                if json { println!("{}", serde_json::to_string_pretty(&ids)?); }
                else { for id in &ids { println!("{id}"); } }
            }
        },
        Commands::Column { cmd } => match cmd {
            ColumnCommands::Create { board_id, title, json } => {
                let mut doc = storage.load_board(&board_id)?;
                let col_id = kanban_core::column::create_column(&mut doc, &title)?;
                storage.save_board(&board_id, &mut doc)?;
                if json { println!("{}", serde_json::json!({"id": col_id, "board_id": board_id})); }
                else { println!("Created column: {title} ({col_id})"); }
            }
        },
        Commands::Card { cmd } => match cmd {
            CardCommands::Create { board_id, col_id, title, json } => {
                let mut doc = storage.load_board(&board_id)?;
                let card = kanban_core::card::create_card(&mut doc, &col_id, &title)?;
                storage.save_board(&board_id, &mut doc)?;
                if json {
                    println!("{}", serde_json::json!({"id": card.id, "title": card.title, "board_id": board_id}));
                } else {
                    println!("Created card: {} ({})", card.title, card.id);
                }
            }
            CardCommands::View { board_id, card_id, json } => {
                let doc = storage.load_board(&board_id)?;
                let card_obj = kanban_core::card::get_card_obj(&doc, &card_id)?;
                let title = kanban_core::get_string(&doc, &card_obj, "title")?.unwrap_or_default();
                if json { println!("{}", serde_json::json!({"id": card_id, "title": title})); }
                else { println!("{}: {}", card_id, title); }
            }
        },
    }
    Ok(())
}
```

- [ ] **Step 3: Run build**

```bash
cargo build -p kanban-cli 2>&1
```
Expected: builds successfully.

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-net/ crates/kanban-cli/
git commit -m "feat(net,cli): kanban-net stub + kanban-cli skeleton with board/column/card commands"
```

---

### Task 5: `kanban-tauri` minimal skeleton + workspace build

**Files:**
- Create: `crates/kanban-tauri/src-tauri/Cargo.toml`
- Create: `crates/kanban-tauri/src-tauri/src/main.rs`
- Create: `crates/kanban-tauri/src-tauri/tauri.conf.json`
- Create: `crates/kanban-tauri/src-tauri/capabilities/default.json`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "kanban-tauri"
version = "0.1.0"
edition = "2021"

[lib]
name = "kanban_tauri_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[[bin]]
name = "kanban-tauri"
path = "src/main.rs"

[dependencies]
kanban-core    = { path = "../../kanban-core" }
kanban-crypto  = { path = "../../kanban-crypto" }
kanban-storage = { path = "../../kanban-storage" }
tauri          = { version = "2", features = [] }
serde          = { workspace = true }
serde_json     = { workspace = true }
thiserror      = { workspace = true }
tracing        = { workspace = true }
tokio          = { workspace = true }
anyhow         = { workspace = true }

[build-dependencies]
tauri-build = { version = "2", features = [] }
```

- [ ] **Step 2: Create `tauri.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "P2P Kanban",
  "version": "0.1.0",
  "identifier": "dev.p2p-kanban",
  "build": {
    "frontendDist": "../src"
  },
  "app": {
    "windows": [{ "title": "P2P Kanban", "width": 1200, "height": 800 }],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": []
  }
}
```

- [ ] **Step 3: Create minimal `src/main.rs`**

```rust
// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Create `crates/kanban-tauri/src-tauri/build.rs`:
```rust
fn main() {
    tauri_build::build()
}
```

Create `crates/kanban-tauri/src-tauri/capabilities/default.json`:
```json
{
  "$schema": "https://schema.tauri.app/capability/2",
  "identifier": "default",
  "description": "Default capability",
  "windows": ["main"],
  "permissions": ["core:default"]
}
```

Create a minimal frontend placeholder `crates/kanban-tauri/src/index.html`:
```html
<!DOCTYPE html><html><body><h1>P2P Kanban</h1></body></html>
```

- [ ] **Step 4: Build workspace (excluding tauri for CI — it requires platform toolchain)**

```bash
cargo build -p kanban-core -p kanban-crypto -p kanban-storage -p kanban-cli 2>&1
```
Expected: all 4 crates build successfully with 0 errors.

Full workspace build including tauri:
```bash
cargo build --workspace 2>&1 | head -50
```
Expected: succeeds (may need Tauri system dependencies on Linux — `libwebkit2gtk-4.1-dev`).

- [ ] **Step 5: Run all tests**

```bash
cargo test -p kanban-core -p kanban-crypto -p kanban-storage 2>&1
```
Expected: all tests pass.

- [ ] **Step 6: Final commit**

```bash
git add crates/kanban-tauri/
git commit -m "feat(tauri): minimal Tauri v2 skeleton — compiles, no commands yet"
```

---

### Task 6: Smoke test — full CLI flow

- [ ] **Step 1: Write end-to-end test**

Place in `crates/kanban-cli/tests/scaffold_smoke_test.rs` (integration test inside kanban-cli, which already has `tempfile` in dev-deps and `app-cli` as the binary target):

```rust
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
    assert!(col_out.status.success());
    let col: serde_json::Value = serde_json::from_slice(&col_out.stdout).unwrap();
    let col_id = col["id"].as_str().unwrap();

    // Create card
    let card_out = cli(tmp.path(), &["card", "create", board_id, col_id, "Deploy API", "--json"]);
    assert!(card_out.status.success());
    let card: serde_json::Value = serde_json::from_slice(&card_out.stdout).unwrap();
    assert_eq!(card["title"], "Deploy API");

    // List boards
    let list_out = cli(tmp.path(), &["board", "list", "--json"]);
    assert!(list_out.status.success());
    let boards: Vec<String> = serde_json::from_slice(&list_out.stdout).unwrap();
    assert_eq!(boards.len(), 1);
}
```

Note: also add `serde_json = { workspace = true }` to `[dev-dependencies]` in `crates/kanban-cli/Cargo.toml` since the smoke test uses it.

- [ ] **Step 2: Run**

```bash
cargo test -p kanban-cli --test scaffold_smoke_test
```
Expected: pass.

- [ ] **Step 3: Commit**

```bash
git add crates/kanban-cli/tests/
git commit -m "test(integration): scaffold smoke test — board/column/card CLI flow end-to-end"
```
