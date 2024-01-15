# Phase 3 Feature Additions: @Mentions, Card Linking, Peer Presence

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add @mention parsing and indexing, Map-based card linking with split-link repair, and ephemeral peer presence heartbeats — all require stable sync from Phase 2.

**Architecture:** Mentions are parsed from Automerge text fields as changes arrive, indexed locally with a checkpoint. Card links use `Map<card_id, true>` in the Automerge doc (idempotent concurrent adds). Presence uses a distinct `SignedPresence` message routed by the 1-byte type prefix from Phase 2; it never touches the Automerge document.

**Tech Stack:** Rust 2021, `automerge = "0.5"`, `rusqlite`, `iroh`, `iroh-gossip`, `ed25519-dalek`, `regex`, `ciborium`, Tauri v2 channels

**Depends on:** Phase 1 (card numbers), Phase 2 (message type-prefix routing, stable Iroh connection).

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| Create | `crates/kanban-core/src/mention.rs` | Parse `@[alias\|pubkey]` tokens, `@all` expansion |
| Create | `crates/kanban-storage/src/mention.rs` | `mention_index` table, checkpoint, notification queries |
| Modify | `crates/kanban-storage/src/schema.rs` | Add `mention_index`, `meta` tables |
| Create | `crates/kanban-core/src/card_link.rs` | `link_cards`, `unlink_cards`, `list_links`, split-link repair |
| Modify | `crates/kanban-core/src/card.rs` | Add `related: Map<card_id, bool>` field |
| Create | `crates/kanban-net/src/presence.rs` | `PresenceHeartbeat`, `SignedPresence`, heartbeat loop |
| Modify | `crates/kanban-net/src/lib.rs` | Route 0x02 messages to presence handler |
| Create | `crates/kanban-cli/src/commands/mention.rs` | `mentions list`, `mentions mark-read` |
| Create | `crates/kanban-cli/src/commands/card_link.rs` | `card link`, `card unlink`, `card links` |
| Modify | `crates/kanban-cli/src/main.rs` | Register new subcommands |
| Modify | `crates/kanban-tauri/src-tauri/src/commands/card.rs` | Add link/unlink Tauri commands |
| Modify | `crates/kanban-tauri/src-tauri/src/main.rs` | Emit `MentionEvent`, `PresenceEvent` via Channel |

---

### Task 1: Mention token parser

**Files:**
- Create: `crates/kanban-core/src/mention.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// crates/kanban-core/src/mention.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_mention() {
        let tokens = extract_mentions("Hello @[Alice|pk_7xq3m] please review");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].alias, "Alice");
        assert_eq!(tokens[0].pubkey, "pk_7xq3m");
    }

    #[test]
    fn parse_at_all() {
        let tokens = extract_mentions("@[all] urgent update");
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_all);
    }

    #[test]
    fn no_mentions_returns_empty() {
        let tokens = extract_mentions("no mentions here");
        assert!(tokens.is_empty());
    }

    #[test]
    fn multiple_mentions_in_text() {
        let tokens = extract_mentions("@[Alice|pk_abc] and @[Bob|pk_def] please check");
        assert_eq!(tokens.len(), 2);
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-core mention
```
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct MentionToken {
    pub alias: String,
    pub pubkey: String,
    pub is_all: bool,
}

fn mention_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"@\[(?:all|([^\|]+)\|([^\]]+))\]").unwrap()
    })
}

pub fn extract_mentions(text: &str) -> Vec<MentionToken> {
    mention_regex()
        .captures_iter(text)
        .map(|cap| {
            if cap[0].contains("|") {
                MentionToken {
                    alias: cap[1].to_string(),
                    pubkey: cap[2].to_string(),
                    is_all: false,
                }
            } else {
                MentionToken { alias: "all".into(), pubkey: "all".into(), is_all: true }
            }
        })
        .collect()
}

/// Scan a set of changed text fields for mentions of `local_pubkey`.
/// Returns (card_id, context, mentioning_pubkey, hlc) for each match.
pub fn find_mentions_for_peer<'a>(
    changes: &'a [ChangedTextField],
    local_pubkey: &str,
) -> Vec<MentionHit<'a>> {
    let mut hits = Vec::new();
    for change in changes {
        let tokens = extract_mentions(&change.text);
        for token in tokens {
            if token.is_all || token.pubkey == local_pubkey {
                hits.push(MentionHit {
                    card_id: &change.card_id,
                    context: change.context.clone(),
                    mentioned_by: change.author_pubkey.clone(),
                    hlc: change.hlc.clone(),
                });
            }
        }
    }
    hits
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-core mention
```
Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-core/src/mention.rs crates/kanban-core/src/lib.rs
git commit -m "feat(core): add @[alias|pubkey] mention token parser"
```

---

### Task 2: `mention_index` SQLite table with incremental checkpoint

**Files:**
- Modify: `crates/kanban-storage/src/schema.rs`
- Create: `crates/kanban-storage/src/mention.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// crates/kanban-storage/src/mention.rs
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
    fn insert_and_list_unread_mentions() {
        let conn = test_db();
        insert_mention(&conn, "board1", "card1", "pk_local", "pk_author", "description", "hlc1").unwrap();
        let mentions = list_mentions(&conn, "pk_local", Some("board1"), true).unwrap();
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].card_id, "card1");
        assert!(!mentions[0].seen);
    }

    #[test]
    fn mark_all_read() {
        let conn = test_db();
        insert_mention(&conn, "board1", "card1", "pk_local", "pk_author", "description", "hlc1").unwrap();
        mark_all_read(&conn, "pk_local", Some("board1")).unwrap();
        let unread = list_mentions(&conn, "pk_local", Some("board1"), true).unwrap();
        assert!(unread.is_empty());
    }

    #[test]
    fn checkpoint_round_trip() {
        let conn = test_db();
        set_mention_scan_checkpoint(&conn, "board1", "hash_abc123").unwrap();
        let cp = get_mention_scan_checkpoint(&conn, "board1").unwrap();
        assert_eq!(cp, Some("hash_abc123".to_string()));
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-storage mention
```
Expected: FAIL.

- [ ] **Step 3: Add SQL migrations**

In `crates/kanban-storage/src/schema.rs`:
```sql
-- mention_index
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

-- meta table for checkpoints and other board-scoped key-value state
CREATE TABLE IF NOT EXISTS meta (
    board_id TEXT NOT NULL,
    key      TEXT NOT NULL,
    value    TEXT NOT NULL,
    PRIMARY KEY (board_id, key)
);
```

- [ ] **Step 4: Implement functions**

```rust
// crates/kanban-storage/src/mention.rs

use rusqlite::{Connection, params};

pub struct MentionRecord {
    pub board_id: String,
    pub card_id: String,
    pub mentioned: String,
    pub mentioned_by: String,
    pub context: String,
    pub hlc: String,
    pub seen: bool,
}

pub fn insert_mention(
    conn: &Connection,
    board_id: &str,
    card_id: &str,
    mentioned: &str,
    mentioned_by: &str,
    context: &str,
    hlc: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO mention_index
         (board_id, card_id, mentioned, mentioned_by, context, hlc, seen)
         VALUES (?1,?2,?3,?4,?5,?6,0)",
        params![board_id, card_id, mentioned, mentioned_by, context, hlc],
    )?;
    Ok(())
}

pub fn list_mentions(
    conn: &Connection,
    mentioned: &str,
    board_id: Option<&str>,
    unread_only: bool,
) -> rusqlite::Result<Vec<MentionRecord>> {
    let sql = match (board_id.is_some(), unread_only) {
        (true, true) =>
            "SELECT board_id,card_id,mentioned,mentioned_by,context,hlc,seen
             FROM mention_index
             WHERE mentioned=?1 AND board_id=?2 AND seen=0
             ORDER BY seen ASC, hlc DESC",
        (true, false) =>
            "SELECT board_id,card_id,mentioned,mentioned_by,context,hlc,seen
             FROM mention_index
             WHERE mentioned=?1 AND board_id=?2
             ORDER BY seen ASC, hlc DESC",
        (false, true) =>
            "SELECT board_id,card_id,mentioned,mentioned_by,context,hlc,seen
             FROM mention_index
             WHERE mentioned=?1 AND seen=0
             ORDER BY hlc DESC",
        (false, false) =>
            "SELECT board_id,card_id,mentioned,mentioned_by,context,hlc,seen
             FROM mention_index
             WHERE mentioned=?1
             ORDER BY seen ASC, hlc DESC",
    };

    let map_row = |r: &rusqlite::Row| -> rusqlite::Result<MentionRecord> {
        Ok(MentionRecord {
            board_id:     r.get(0)?,
            card_id:      r.get(1)?,
            mentioned:    r.get(2)?,
            mentioned_by: r.get(3)?,
            context:      r.get(4)?,
            hlc:          r.get(5)?,
            seen:         r.get::<_, i32>(6)? != 0,
        })
    };

    match board_id {
        Some(b) => {
            let mut stmt = conn.prepare(sql)?;
            stmt.query_map(params![mentioned, b], map_row)?
                .collect::<rusqlite::Result<Vec<_>>>()
        }
        None => {
            let mut stmt = conn.prepare(sql)?;
            stmt.query_map(params![mentioned], map_row)?
                .collect::<rusqlite::Result<Vec<_>>>()
        }
    }
}

pub fn mark_all_read(conn: &Connection, mentioned: &str, board_id: Option<&str>) -> rusqlite::Result<()> {
    match board_id {
        Some(b) => conn.execute(
            "UPDATE mention_index SET seen=1 WHERE mentioned=?1 AND board_id=?2",
            params![mentioned, b],
        )?,
        None => conn.execute(
            "UPDATE mention_index SET seen=1 WHERE mentioned=?1",
            params![mentioned],
        )?,
    };
    Ok(())
}

pub fn get_mention_scan_checkpoint(conn: &Connection, board_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM meta WHERE board_id=?1 AND key='mention_scan_checkpoint'",
        params![board_id],
        |r| r.get(0),
    ).optional()
}

pub fn set_mention_scan_checkpoint(conn: &Connection, board_id: &str, hash: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (board_id, key, value) VALUES (?1,'mention_scan_checkpoint',?2)
         ON CONFLICT(board_id,key) DO UPDATE SET value=excluded.value",
        params![board_id, hash],
    )?;
    Ok(())
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p kanban-storage mention
```
Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/kanban-storage/src/mention.rs crates/kanban-storage/src/schema.rs
git commit -m "feat(storage): mention_index with incremental checkpoint"
```

---

### Task 3: Wire mention scanning into the Automerge merge path

**Files:**
- Modify: `crates/kanban-storage/src/lib.rs`

After each Automerge merge (local or peer), scan new text fields for mentions of the local user and upsert into `mention_index`. Update the checkpoint.

- [ ] **Step 1: Write the test**

```rust
#[test]
fn mention_index_populated_after_peer_change() {
    let mut storage = build_test_storage();
    let local_pk = storage.load_identity().unwrap().public_key_hex();

    // Simulate a peer change containing "@[LocalUser|<local_pk>]" in a card description
    let peer_changes = build_peer_change_with_mention(&local_pk);
    storage.merge_board_changes("board1", &peer_changes).unwrap();

    let mentions = storage.list_mentions(&local_pk, Some("board1"), true).unwrap();
    assert_eq!(mentions.len(), 1);
}
```

- [ ] **Step 2: Implement**

In `merge_board_changes` (or equivalent), after the Automerge merge:
```rust
// Extract changed text fields from the diff
let changed_fields = extract_changed_text_fields(&doc, &prev_heads);
let hits = kanban_core::mention::find_mentions_for_peer(&changed_fields, &local_pubkey);
for hit in hits {
    kanban_storage::mention::insert_mention(
        &self.conn, board_id, &hit.card_id, &local_pubkey,
        &hit.mentioned_by, &hit.context, &hit.hlc,
    )?;
}
// Update checkpoint to the latest change hash
let latest_hash = doc.get_heads().first().map(|h| hex::encode(h.0)).unwrap_or_default();
kanban_storage::mention::set_mention_scan_checkpoint(&self.conn, board_id, &latest_hash)?;
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p kanban-storage mention_index_populated
```
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-storage/src/lib.rs
git commit -m "feat(storage): scan mentions after every Automerge merge and update checkpoint"
```

---

### Task 4: Card linking (`Map<card_id, true>`)

**Files:**
- Create: `crates/kanban-core/src/card_link.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// crates/kanban-core/src/card_link.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_creates_bidirectional_entries() {
        let mut doc = build_test_board_with_two_cards("c1", "c2");
        link_cards(&mut doc, "c1", "c2").unwrap();
        assert!(is_linked(&doc, "c1", "c2").unwrap());
        assert!(is_linked(&doc, "c2", "c1").unwrap()); // bidirectional
    }

    #[test]
    fn link_is_idempotent() {
        let mut doc = build_test_board_with_two_cards("c1", "c2");
        link_cards(&mut doc, "c1", "c2").unwrap();
        link_cards(&mut doc, "c1", "c2").unwrap(); // second call is no-op
        let links = list_links(&doc, "c1").unwrap();
        assert_eq!(links.len(), 1); // not duplicated
    }

    #[test]
    fn unlink_removes_both_sides() {
        let mut doc = build_test_board_with_two_cards("c1", "c2");
        link_cards(&mut doc, "c1", "c2").unwrap();
        unlink_cards(&mut doc, "c1", "c2").unwrap();
        assert!(!is_linked(&doc, "c1", "c2").unwrap());
        assert!(!is_linked(&doc, "c2", "c1").unwrap());
    }

    #[test]
    fn cross_board_dangling_ref_renders_gracefully() {
        let doc = build_test_board_with_two_cards("c1", "c2");
        // c3 doesn't exist in this doc
        let display = resolve_link_display(&doc, "c1", "nonexistent-card-id");
        assert!(display.contains("not found"));
    }

    #[test]
    fn split_link_repair_detects_and_queues() {
        // c1.related has c2, but c2.related does NOT have c1
        let mut doc = build_split_link_state("c1", "c2");
        let splits = detect_split_links(&doc).unwrap();
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0], ("c1".to_string(), "c2".to_string()));
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-core card_link
```
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
// crates/kanban-core/src/card_link.rs

use automerge::{AutoCommit, ObjType, ReadDoc, transaction::Transactable};

fn get_related_map(doc: &mut AutoCommit, card_id: &str) -> Result<automerge::ObjId, crate::Error> {
    let cards_map = crate::card::get_cards_map(doc)?;
    let card_obj = doc.get(&cards_map, card_id)?
        .ok_or_else(|| crate::Error::NotFound(card_id.into()))?.1;
    match doc.get(&card_obj, "related")? {
        Some((_, id)) => Ok(id),
        None => Ok(doc.put_object(&card_obj, "related", ObjType::Map)?),
    }
}

pub fn link_cards(doc: &mut AutoCommit, card_a: &str, card_b: &str) -> Result<(), crate::Error> {
    let rel_a = get_related_map(doc, card_a)?;
    let rel_b = get_related_map(doc, card_b)?;
    // Map key = card_id, value = true — idempotent by Automerge map semantics
    doc.put(&rel_a, card_b, true)?;
    doc.put(&rel_b, card_a, true)?;
    Ok(())
}

pub fn unlink_cards(doc: &mut AutoCommit, card_a: &str, card_b: &str) -> Result<(), crate::Error> {
    let rel_a = get_related_map(doc, card_a)?;
    let rel_b = get_related_map(doc, card_b)?;
    let _ = doc.delete(&rel_a, card_b);
    let _ = doc.delete(&rel_b, card_a);
    Ok(())
}

pub fn is_linked(doc: &AutoCommit, card_a: &str, card_b: &str) -> Result<bool, crate::Error> {
    let cards_map = crate::card::get_cards_map_readonly(doc)?;
    let card_obj = match doc.get(&cards_map, card_a)? {
        Some((_, id)) => id,
        None => return Ok(false),
    };
    let related = match doc.get(&card_obj, "related")? {
        Some((_, id)) => id,
        None => return Ok(false),
    };
    Ok(doc.get(&related, card_b)?.is_some())
}

pub fn list_links(doc: &AutoCommit, card_id: &str) -> Result<Vec<String>, crate::Error> {
    let cards_map = crate::card::get_cards_map_readonly(doc)?;
    let card_obj = match doc.get(&cards_map, card_id)? {
        Some((_, id)) => id,
        None => return Ok(vec![]),
    };
    let related = match doc.get(&card_obj, "related")? {
        Some((_, id)) => id,
        None => return Ok(vec![]),
    };
    Ok(doc.keys(&related).map(|k| k.to_string()).collect())
}

pub fn resolve_link_display(doc: &AutoCommit, _card_id: &str, target_id: &str) -> String {
    // Try to get the target card's number and title
    match crate::card::get_card_display_name(doc, target_id) {
        Ok(Some(name)) => name,
        _ => format!("[card not found — {}]", &target_id[..8.min(target_id.len())]),
    }
}

/// Detect (card_a, card_b) pairs where A→B exists but B→A does not.
pub fn detect_split_links(doc: &AutoCommit) -> Result<Vec<(String, String)>, crate::Error> {
    let cards_map = crate::card::get_cards_map_readonly(doc)?;
    let mut splits = Vec::new();
    for card_id in doc.keys(&cards_map) {
        let links = list_links(doc, card_id)?;
        for target_id in links {
            if !is_linked(doc, &target_id, card_id)? {
                splits.push((card_id.to_string(), target_id));
            }
        }
    }
    Ok(splits)
}

/// Run the periodic split-link repair pass (at most once per 60s per board, with jitter).
/// Returns number of splits repaired.
pub fn repair_split_links(doc: &mut AutoCommit) -> Result<usize, crate::Error> {
    let splits = detect_split_links(doc)?;
    let count = splits.len();
    for (a, b) in splits {
        // Re-add the missing reverse entry only
        let rel_b = get_related_map(doc, &b)?;
        doc.put(&rel_b, &a, true)?;
    }
    Ok(count)
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-core card_link
```
Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-core/src/card_link.rs crates/kanban-core/src/lib.rs
git commit -m "feat(core): Map-based card linking with split-link repair"
```

---

### Task 5: Periodic split-link repair scheduler

**Files:**
- Modify: `crates/kanban-storage/src/lib.rs` (or a background task in `kanban-tauri`)

The repair runs at most once every 60 seconds per board, with a random jitter of 0–30s to avoid peer storms.

- [ ] **Step 1: Write the test**

```rust
#[test]
fn repair_not_run_twice_within_60s() {
    let storage = build_test_storage();
    assert!(storage.should_run_link_repair("board1").unwrap());
    storage.record_link_repair_run("board1").unwrap();
    assert!(!storage.should_run_link_repair("board1").unwrap()); // within 60s window
}
```

- [ ] **Step 2: Implement**

Add to the `meta` table schema (already created in Task 2):
```sql
-- meta row: ('board_id', 'last_link_repair', '<timestamp>')
```

```rust
pub fn should_run_link_repair(conn: &Connection, board_id: &str) -> rusqlite::Result<bool> {
    let last: Option<String> = conn.query_row(
        "SELECT value FROM meta WHERE board_id=?1 AND key='last_link_repair'",
        params![board_id], |r| r.get(0),
    ).optional()?;
    match last {
        None => Ok(true),
        Some(ts) => {
            let elapsed = chrono::Utc::now()
                .signed_duration_since(chrono::DateTime::parse_from_rfc3339(&ts).unwrap())
                .num_seconds();
            Ok(elapsed >= 60)
        }
    }
}

pub fn record_link_repair_run(conn: &Connection, board_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (board_id,key,value) VALUES (?1,'last_link_repair',?2)
         ON CONFLICT(board_id,key) DO UPDATE SET value=excluded.value",
        params![board_id, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}
```

In the Tokio background task (called from `kanban-tauri` or the daemon):
```rust
async fn link_repair_loop(board_id: String, storage: Arc<Mutex<Storage>>, doc: Arc<Mutex<AutoCommit>>) {
    loop {
        // Random jitter: 0–30 seconds
        let jitter = rand::thread_rng().gen_range(0u64..30);
        tokio::time::sleep(Duration::from_secs(jitter)).await;

        let mut storage = storage.lock().await;
        if storage.should_run_link_repair(&board_id).unwrap_or(false) {
            let mut doc = doc.lock().await;
            let _ = kanban_core::card_link::repair_split_links(&mut doc);
            storage.record_link_repair_run(&board_id).ok();
        }
        tokio::time::sleep(Duration::from_secs(60 - jitter)).await;
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p kanban-storage repair_not_run
```
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-storage/src/lib.rs
git commit -m "feat(storage): periodic split-link repair with 60s interval and jitter guard"
```

---

### Task 6: Peer presence heartbeats

**Files:**
- Create: `crates/kanban-net/src/presence.rs`
- Modify: `crates/kanban-net/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// crates/kanban-net/src/presence.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_encodes_and_decodes() {
        let keypair = ed25519_dalek::SigningKey::generate(&mut rand::thread_rng());
        let hb = PresenceHeartbeat {
            board_id: "board1".into(),
            focused_card: Some("card-uuid-1".into()),
            hlc: "2026-03-19T00:00:00Z".into(),
        };
        let signed = sign_presence(&keypair, &hb).unwrap();
        let (verified_hb, pubkey) = verify_and_decode_presence(&signed).unwrap();
        assert_eq!(verified_hb.board_id, "board1");
        assert_eq!(pubkey, keypair.verifying_key());
    }

    #[test]
    fn stale_presence_entry_expires() {
        let mut tracker = PresenceTracker::new();
        let pk = [1u8; 32];
        tracker.update(pk, PresenceHeartbeat {
            board_id: "board1".into(),
            focused_card: None,
            hlc: "2026-03-19T00:00:00Z".into(),
        }, std::time::Instant::now() - std::time::Duration::from_secs(20));
        let active = tracker.active_peers("board1");
        assert!(active.is_empty()); // TTL is 15s, 20s elapsed
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-net presence
```
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
// crates/kanban-net/src/presence.rs

use ed25519_dalek::{SigningKey, Signature, Signer, Verifier, VerifyingKey};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const PRESENCE_TTL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PresenceHeartbeat {
    pub board_id: String,
    pub focused_card: Option<String>,
    pub hlc: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SignedPresence {
    pub payload: Vec<u8>,   // CBOR-encoded PresenceHeartbeat
    pub author: Vec<u8>,    // 32-byte Ed25519 public key
    pub signature: Vec<u8>, // 64-byte signature
}

pub fn sign_presence(keypair: &SigningKey, hb: &PresenceHeartbeat) -> Result<SignedPresence, crate::Error> {
    let payload = ciborium::ser::into_vec(hb).map_err(|_| crate::Error::Serialization)?;
    let sig: Signature = keypair.sign(&payload);
    Ok(SignedPresence {
        payload,
        author: keypair.verifying_key().to_bytes().to_vec(),
        signature: sig.to_bytes().to_vec(),
    })
}

pub fn verify_and_decode_presence(msg: &SignedPresence) -> Result<(PresenceHeartbeat, VerifyingKey), crate::Error> {
    let pubkey = VerifyingKey::from_bytes(
        msg.author.as_slice().try_into().map_err(|_| crate::Error::InvalidKey)?,
    ).map_err(|_| crate::Error::InvalidKey)?;
    let sig = Signature::from_bytes(
        msg.signature.as_slice().try_into().map_err(|_| crate::Error::InvalidSignature)?,
    );
    pubkey.verify(&msg.payload, &sig).map_err(|_| crate::Error::InvalidSignature)?;
    let hb: PresenceHeartbeat = ciborium::de::from_reader(&msg.payload[..])
        .map_err(|_| crate::Error::Serialization)?;
    Ok((hb, pubkey))
}

pub struct PresenceEntry {
    pub heartbeat: PresenceHeartbeat,
    pub last_seen: Instant,
}

pub struct PresenceTracker {
    entries: HashMap<[u8; 32], PresenceEntry>,
}

impl PresenceTracker {
    pub fn new() -> Self { Self { entries: HashMap::new() } }

    pub fn update(&mut self, pubkey: [u8; 32], hb: PresenceHeartbeat, now: Instant) {
        self.entries.insert(pubkey, PresenceEntry { heartbeat: hb, last_seen: now });
    }

    pub fn active_peers(&mut self, board_id: &str) -> Vec<([u8; 32], &PresenceHeartbeat)> {
        let now = Instant::now();
        self.entries.retain(|_, e| now.duration_since(e.last_seen) < PRESENCE_TTL);
        self.entries.iter()
            .filter(|(_, e)| e.heartbeat.board_id == board_id)
            .map(|(pk, e)| (*pk, &e.heartbeat))
            .collect()
    }
}
```

- [ ] **Step 4: Route 0x02 messages to presence handler**

In `crates/kanban-net/src/lib.rs`, in the gossip message receive loop:
```rust
let (msg_type, body) = crate::message::decode_message(&raw_bytes)?;
match msg_type {
    MessageType::AutomergeChange => {
        // existing Automerge apply path
    }
    MessageType::PresenceHeartbeat => {
        let signed: SignedPresence = ciborium::de::from_reader(body)?;
        if let Ok((hb, pubkey)) = verify_and_decode_presence(&signed) {
            // Check pubkey is a known board member before accepting
            if board_members.contains(&pubkey.to_bytes()) {
                presence_tracker.lock().await.update(
                    pubkey.to_bytes(),
                    hb,
                    Instant::now(),
                );
                // Emit presence update via Tauri channel
                app_handle.emit("presence-update", PresenceUpdatePayload { ... })?;
            }
        }
    }
    MessageType::VersionHandshake => { /* handled at connection time */ }
}
```

- [ ] **Step 5: Start heartbeat broadcast loop**

```rust
async fn presence_broadcast_loop(
    board_id: String,
    keypair: SigningKey,
    gossip: iroh_gossip::Gossip,
    focused_card: Arc<Mutex<Option<String>>>,
) {
    loop {
        let card = focused_card.lock().await.clone();
        let hb = PresenceHeartbeat { board_id: board_id.clone(), focused_card: card, hlc: hlc_now() };
        if let Ok(signed) = sign_presence(&keypair, &hb) {
            if let Ok(payload) = ciborium::ser::into_vec(&signed) {
                let msg = crate::message::encode_message(MessageType::PresenceHeartbeat, &payload);
                let _ = gossip.broadcast(board_id_to_topic(&board_id), msg.into()).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p kanban-net presence
```
Expected: both tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/kanban-net/src/presence.rs crates/kanban-net/src/lib.rs
git commit -m "feat(net): SignedPresence heartbeat with TTL tracker and 0x02 type routing"
```

---

### Task 7: Mention + card link CLI commands

**Files:**
- Create: `crates/kanban-cli/src/commands/mention.rs`
- Create: `crates/kanban-cli/src/commands/card_link.rs`
- Modify: `crates/kanban-cli/src/main.rs`

- [ ] **Step 1: Write integration tests**

```rust
#[test]
fn mention_list_shows_unread() {
    // Add a card with @[LocalUser|pk] in description via a peer change
    // Run: app-cli mentions list --board <id> --unread --json
    // Verify output contains the mention
}

#[test]
fn card_link_and_unlink() {
    // Create two cards
    // app-cli card link <board> <c1> <c2> --json
    // app-cli card links <board> <c1> --json → verify c2 in list
    // app-cli card unlink <board> <c1> <c2> --json
    // app-cli card links <board> <c1> --json → verify empty
}
```

- [ ] **Step 2: Implement `mention.rs`**

```rust
use clap::Subcommand;

#[derive(Subcommand)]
pub enum MentionCommand {
    List {
        #[arg(long)] board: Option<String>,
        #[arg(long)] unread: bool,
        #[arg(long)] json: bool,
    },
    MarkRead {
        #[arg(long)] board: Option<String>,
        #[arg(long)] json: bool,
    },
}

pub fn run(storage: &mut Storage, cmd: MentionCommand) -> anyhow::Result<()> {
    match cmd {
        MentionCommand::List { board, unread, json } => {
            let identity = storage.load_identity()?;
            let pk = identity.public_key_hex();
            let mentions = storage.mention_index()
                .list_mentions(&pk, board.as_deref(), unread)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&mentions)?);
            } else {
                for m in &mentions {
                    let status = if m.seen { "" } else { " [unread]" };
                    println!("{} in {}/{}{}", m.mentioned_by, m.board_id, m.card_id, status);
                }
            }
        }
        MentionCommand::MarkRead { board, json } => {
            let identity = storage.load_identity()?;
            storage.mention_index().mark_all_read(&identity.public_key_hex(), board.as_deref())?;
            if json { println!("{{\"ok\":true}}"); } else { println!("Marked all mentions as read."); }
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Implement `card_link.rs`**

```rust
use clap::Subcommand;

#[derive(Subcommand)]
pub enum CardLinkCommand {
    Link { board_id: String, card_id: String, target_card_id: String, #[arg(long)] json: bool },
    Unlink { board_id: String, card_id: String, target_card_id: String, #[arg(long)] json: bool },
    Links { board_id: String, card_id: String, #[arg(long)] json: bool },
}

pub fn run(storage: &mut Storage, cmd: CardLinkCommand) -> anyhow::Result<()> {
    match cmd {
        CardLinkCommand::Link { board_id, card_id, target_card_id, json } => {
            let target_uuid = storage.resolve_card_ref(&board_id, &target_card_id)?;
            let mut doc = storage.load_board(&board_id)?;
            kanban_core::card_link::link_cards(&mut doc, &card_id, &target_uuid)?;
            storage.save_board(&board_id, &doc)?;
            if json { println!("{{\"ok\":true}}"); } else { println!("Linked."); }
        }
        CardLinkCommand::Unlink { board_id, card_id, target_card_id, json } => {
            let target_uuid = storage.resolve_card_ref(&board_id, &target_card_id)?;
            let mut doc = storage.load_board(&board_id)?;
            kanban_core::card_link::unlink_cards(&mut doc, &card_id, &target_uuid)?;
            storage.save_board(&board_id, &doc)?;
            if json { println!("{{\"ok\":true}}"); } else { println!("Unlinked."); }
        }
        CardLinkCommand::Links { board_id, card_id, json } => {
            let doc = storage.load_board(&board_id)?;
            let links = kanban_core::card_link::list_links(&doc, &card_id)?;
            let display: Vec<String> = links.iter()
                .map(|id| kanban_core::card_link::resolve_link_display(&doc, &card_id, id))
                .collect();
            if json { println!("{}", serde_json::to_string_pretty(&display)?); }
            else { for l in &display { println!("{}", l); } }
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-cli mention card_link
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-cli/src/commands/mention.rs crates/kanban-cli/src/commands/card_link.rs crates/kanban-cli/src/main.rs
git commit -m "feat(cli): mentions list/mark-read and card link/unlink/links commands"
```

---

### Task 8: End-to-end smoke test

- [ ] **Step 1: Write integration test**

```rust
// tests/integration/phase3_smoke_test.rs

#[test]
fn mentions_received_from_peer() {
    // Two in-process nodes sharing a board
    // Node B edits card description to include @[NodeA|pk_a]
    // NodeA's mention index is updated
    // app-cli mentions list --unread shows the mention
}

#[test]
fn card_links_converge_after_concurrent_add() {
    // Two nodes both add the same link concurrently
    // After merge: link appears exactly once on each side
}

#[test]
fn presence_visible_after_heartbeat() {
    // Node B starts broadcasting heartbeats
    // Node A's presence tracker shows Node B within 5s
}
```

- [ ] **Step 2: Run**

```bash
cargo test --test phase3_smoke_test
```

- [ ] **Step 3: Commit**

```bash
git add tests/integration/phase3_smoke_test.rs
git commit -m "test(integration): phase3 smoke tests for mentions, card links, presence"
```
