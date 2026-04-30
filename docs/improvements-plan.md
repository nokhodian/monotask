# Monotask Improvement Plan

*Generated from a 5-agent audit: UI/UX, P2P networking, storage/performance, feature gaps, and code quality.*

---

## Overview

The app is functionally solid — Automerge CRDT sync, libp2p networking, and the Tauri shell all work. The main gaps are: **stability risks** (mutex panics, memory leaks), **performance bottlenecks** (N+1 queries, uncompacted docs), and a **feature shortlist** that will make the app feel complete.

---

## CRITICAL — Fix Before Next Release

These can cause crashes, data loss, or security vulnerabilities.

### C1. Mutex `.unwrap()` → crash on panic
**Files:** `main.rs:140, 171, 177, 184, 1356`

Five places call `.lock().unwrap()` on `AppState` mutexes. If any operation inside a mutex guard panics (e.g., a DB error), the mutex becomes poisoned and **all subsequent lock attempts crash the app**.

```rust
// Current (crashes if poisoned)
state.storage.lock().unwrap()

// Fix
state.storage.lock().map_err(|e| e.to_string())?
```

### C2. `unchecked_transaction()` — potential DB corruption
**File:** `crates/kanban-storage/src/board.rs:6`

Uses `unchecked_transaction()` which bypasses proper rollback-on-error. Replace with `conn.transaction()` (checked).

### C3. Leaked `setInterval` handles — memory/CPU leak
**File:** `crates/kanban-tauri/src/index.html:3656, 3689`

Two intervals are started unconditionally and never stored or cleared:
```javascript
setInterval(pollSyncStatus, 4000);      // line 3656 — never cleared
setInterval(checkForUpdate, 3600000);   // line 3689 — never cleared
```
Store the handles and add cleanup paths.

### C4. XSS via unescaped `innerHTML` with peer-controlled data
**File:** `crates/kanban-tauri/src/index.html:3053, 3065, 3080, 3113, 3315, 3347`

Several `innerHTML` assignments use user/peer-controlled data (display names, avatar URLs, color accents) without calling `escapeHtml()`. A malicious peer can craft a `display_name` or `color_accent` that injects HTML/JS.

```javascript
// Vulnerable (avatar_b64 and color_accent from peer, never escaped)
`background-image:url(data:image/png;base64,${p.avatar_b64})`

// Fix: validate base64 server-side; use escapeHtml() on all string insertions
```

Audit every `innerHTML` assignment and apply `escapeHtml()` consistently.

### C5. No message size limit in sync protocol — DoS
**File:** `crates/kanban-net/src/sync_protocol.rs:63-66`

```rust
let mut buf = vec![0u8; len as usize];  // len comes from peer — no limit!
io.read_exact(&mut buf).await?;
```

A malicious peer sends `len = 1_073_741_824` (1 GB) and the node allocates 1 GB of RAM before reading a byte.

**Fix:** Add `const MAX_MSG_SIZE: u32 = 10 * 1024 * 1024;` and reject messages above it.

### C6. P2P event channel never polled
**From networking audit:** The `event_rx` channel on `NetworkHandle` is never polled in the Tauri main loop. `NetEvent::BoardSynced`, `PeerConnected`, and `PeerDisconnected` events are produced but silently dropped. UI never learns about completed syncs without polling.

**Fix:** Spawn a task in `main.rs` that reads `event_rx` and emits Tauri events to the frontend.

---

## HIGH — Target Next 2 Sprints

### H1. N+1 query in `list_boards` and `get_sync_info_cmd`
**Files:** `main.rs:350-362`, `main.rs:1434-1445`

Both load the full Automerge doc for every board just to read the title. With 50 boards this is 51 SQLite round-trips, each deserializing potentially large BLOBs.

**Fix:** Add a `title TEXT` column to the `boards` table (schema v4 migration). Populate it on save. Read by SQL, not by deserializing the doc.

### H2. Missing database indexes
**File:** `crates/kanban-storage/src/schema.rs`

Three indexes are missing, causing full table scans on common queries:

```sql
-- Boards listing (filter + sort)
CREATE INDEX IF NOT EXISTS idx_boards_system_modified
  ON boards (is_system, last_modified DESC);

-- Card search lookup by card_id + space_id
CREATE INDEX IF NOT EXISTS idx_card_search_card_space
  ON card_search_index (card_id, space_id);

-- space_boards foreign key lookup
CREATE INDEX IF NOT EXISTS idx_space_boards_board_id
  ON space_boards (board_id);
```

Add in schema v4 migration.

### H3. Input validation missing on all Tauri commands
**File:** `main.rs` — `create_card_cmd` (601), `create_space` (218), `add_comment_cmd` (723), `create_column_cmd` (512), `set_card_title_cmd` (687)

No empty-string or length checks. A client (or peer) can create cards with blank titles and descriptions with 100MB of text.

```rust
fn validate_non_empty(s: &str, field: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err(format!("{} cannot be empty", field));
    }
    if s.len() > 100_000 {
        return Err(format!("{} is too long", field));
    }
    Ok(())
}
```

### H4. Unbounded `sync_states` HashMap — memory leak
**File:** `crates/kanban-net/src/swarm.rs:109`

`sync_states: HashMap<String, automerge::sync::State>` grows a new entry for every board × peer combination and is never pruned. In long-running sessions this leaks memory.

**Fix:** Remove stale entries when a peer disconnects, or limit to the 100 most recently active.

### H5. Automerge doc compaction
No compaction strategy anywhere. Docs grow unboundedly with operation history. A board with months of edits can reach 50 MB+, making sync transfers slow.

**Fix:** After saving, check `doc.save().len() > 10_000_000` and call a compact cycle. Expose a periodic background compaction in the storage layer.

### H6. No authorization checks on destructive commands
**File:** `main.rs` — delete commands

Any client that knows a `board_id` or `space_id` can delete it. There's no check that the caller is the owner or a member of the space.

**Fix:** Add ownership/membership verification in `delete_space`, `delete_board`, `delete_card` before executing.

---

## MEDIUM — Quality & UX Improvements

### M1. Split `main.rs` (1,896 lines) into modules
Organize by domain:
- `commands/cards.rs`
- `commands/boards.rs`
- `commands/spaces.rs`
- `commands/chat.rs`
- `commands/sync.rs`
- `helpers/` for `space_to_view()`, `get_card_labels()`, etc.

### M2. Split `index.html` (3,692 lines) into modules
Extract:
- `css/main.css` — all styles
- `js/cards.js`, `js/spaces.js`, `js/chat.js`, `js/sync.js`

Or at minimum break the `<script>` block into logical sections with clear comments.

### M3. Replace `eprintln!()` with `tracing::debug!()`
`main.rs` and `swarm.rs` have 10+ `eprintln!()` calls that appear in release builds. Switch to `tracing` macros so they compile away in release or can be controlled via env var.

### M4. Fix compiler warnings
```
warning: unused imports: ObjType, ReadDoc (kanban-core, kanban-net)
warning: unused variable: sig_bytes (kanban-crypto)
```
Run `cargo fix --allow-dirty` to clean these.

### M5. Orphaned `card_search_index` rows
When a board is deleted, the `card_search_index` entries for its cards are not cleaned up (no CASCADE DELETE). Search can return phantom cards.

**Fix:** Add `REFERENCES boards(board_id) ON DELETE CASCADE` constraint, or clean up in the delete_board storage function.

### M6. Sync state leakage on reconnect
**From networking audit:** When a peer reconnects, the old `sync::State` for that peer is reused without reset. This can cause sync to send the wrong changes (diff against a stale baseline).

**Fix:** Reset the `sync::State` for a peer on disconnect event.

### M7. P2P reconnection — no exponential backoff
After a disconnect, reconnection attempts are not rate-limited. Rapid reconnect storms can occur on unstable networks.

**Fix:** Add exponential backoff (1s → 2s → 4s → max 60s) on reconnect attempts.

### M8. Missing weak reconnection on new board creation
When a board is created and `trigger_board_sync` is called, connected peers don't learn about the new board in the space doc. Also call `announce_all_spaces` after board creation and deletion to ensure peers get the updated space membership.

---

### H7. Avatar `base64` with no size limit — memory exhaustion
**File:** `main.rs:234-236`

`base64::decode(&profile.avatar_b64).ok()` — `.ok()` silently drops errors and there is no size check. A peer can send a 100 MB avatar string and exhaust the host's RAM.

**Fix:** Reject avatars > 512 KB before decoding.

### H8. 24-hour idle connection timeout wastes resources
**File:** `crates/kanban-net/src/swarm.rs:52`

```rust
.with_idle_connection_timeout(Duration::from_secs(24 * 3600))
```

Keeps dead connections open for a full day. Reduce to 300s with periodic keepalives.

---

## FEATURE GAPS — Prioritized by Impact/Effort

### Quick Wins (Low Effort, High Impact)

| # | Feature | What's Missing | Notes |
|---|---------|----------------|-------|
| F1 | **Card priority** | No priority field (high/medium/low) | Add to Card automerge map + UI dropdown |
| F2 | **Comment editing** | Add/delete only; no edit | Edit flow in card modal |
| F3 | **Message deletion in chat** | Can send, can't delete | Owner-only delete |
| F4 | **Global search UI** | Backend index exists, no UI | Wire `card_search_index` to a search panel (Cmd+K) |
| F5 | **Keyboard shortcuts** | None implemented | Esc to close, Cmd+K search, Enter to add card |
| F6 | **Emoji reactions** | No reactions in chat | Reactions list on messages (👍 ❤️ etc) |
| F7 | **Undo/redo wiring** | DB tables `undo_stack`/`redo_stack` already exist in schema, no Tauri commands | Add `undo_cmd`/`redo_cmd` + keyboard shortcuts (Cmd+Z/Shift+Z) |

### Medium Effort

| # | Feature | Notes |
|---|---------|-------|
| F7 | **Board filters** | Filter by assignee, label, priority; sort cards |
| F8 | **In-app notifications** | @mentions, board activity; needs notification center |
| F9 | **Typing indicators in chat** | Broadcast ephemeral typing state via libp2p pubsub |
| F10 | **Bulk card operations** | Multi-select + bulk reassign/label/archive |
| F11 | **Presence indicators** | Show who's online in sidebar and on cards |
| F12 | **Thread replies in chat** | Nested replies; flat feed gets noisy |

### Higher Effort / Later

| # | Feature | Notes |
|---|---------|-------|
| F13 | **Data export** | Board to JSON/CSV for backup/portability |
| F14 | **Card templates** | Save/reuse checklist + description stubs |
| F15 | **Board templates** | Clone board structure with empty cards |
| F16 | **WIP limits** | Per-column work-in-progress limits |
| F17 | **Due date reminders** | In-app notification when due date passes |
| F18 | **File sharing in chat** | Image paste, file attachments |

---

## TEST COVERAGE GAPS

The storage layer (`space.rs`) has 13 good tests — use that as the model.

Priority missing tests:
1. `test_create_card_empty_title` — validate empty input rejected
2. `test_delete_board_unauthorized` — ensure auth check works
3. `test_storage_mutex_poison_handling` — confirm no panic on poisoned lock
4. `test_network_sync_states_pruned_on_disconnect` — memory leak prevention
5. Integration test: create space → invite peer → create board → verify both see it

---

## Implementation Order Recommendation

**Sprint 1 (Stability + Security):** C1 + C2 + C3 + C4 + C5 + C6 + H3 + H4 + H7
**Sprint 2 (Performance):** H1 + H2 + H5 + H8 + M5
**Sprint 3 (Features):** F1 + F2 + F3 + F4 + F5 + F6 + F7
**Sprint 4 (Architecture):** M1 + M2 + M3 + H6 + M6 + M7
**Sprint 5 (Features cont.):** F8 + F9 + F10 + F11 + F12

---

## Summary Counts

| Category | Critical | High | Medium | Feature |
|----------|----------|------|--------|---------|
| Issues   | 6        | 8    | 8      | 19      |
