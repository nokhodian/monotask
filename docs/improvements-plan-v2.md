# Monotask Improvements Plan v2

Synthesized from a 5-agent swarm audit of the codebase after v1 improvements were merged. Covers fresh issues found in: UI/UX, Feature Completeness, P2P Networking, Architecture/Tech Debt, and Performance/Scalability.

---

## CRITICAL (Fix immediately — correctness/stability)

### C1. N+1 Query in `list_boards_cmd` — 101 DB queries per sidebar load
**File:** `crates/kanban-tauri/src-tauri/src/main.rs:398-410`
**Impact:** ~500-800ms for 50+ boards; scales linearly with board count.

Current code loads the full automerge BLOB for each board just to read the title:
```rust
for id in ids {
    let title = kanban_storage::board::load_board(storage.conn(), &id) // N individual queries
        .ok()
        .and_then(|doc| kanban_core::board::get_board_title(&doc).ok())
        .unwrap_or_else(|| id.clone());
}
```

**Fix:** Cache board titles in a `board_metadata` SQLite column (text, nullable) updated on every `update_board_title_cmd`. `list_boards_cmd` becomes a single `SELECT id, cached_title FROM boards`. Fall back to doc title if NULL.

---

### C2. O(n²) History Scans in `get_board_detail` — 50,000+ doc.get() calls with 100 cards
**File:** `crates/kanban-tauri/src-tauri/src/main.rs:414-474`
**Impact:** 2–5 seconds for boards with 100+ cards on M1/M2.

`get_card_history()` scans the entire history list for every card while loading the board. With 100 cards × 5 moves each = 500+ entries traversed per board load.

**Fix:** Lazy-load card history. Remove history from `CardDetailView` returned by `get_board_detail`. Add a separate `get_card_history_cmd(board_id, card_id)` called only when a card detail modal opens.

---

### C3. `get_sync_status_cmd` is a Hardcoded Stub — Always Returns 0 Peers
**File:** `crates/kanban-tauri/src-tauri/src/main.rs` (search for `get_sync_status_cmd`)
**Impact:** Sync dot and Sync & Peers panel always show 0 peers even when connected.

**Fix:** Replace stub with `state.net.lock()?.get_peer_count()` (or equivalent). Implement `get_peer_count()` on `NetworkHandle` that returns the count from the swarm's connected peers map.

---

### C4. Missing "Leave Space" for Non-Owners — No Exit Path
**Files:** `crates/kanban-tauri/src-tauri/src/main.rs`, `crates/kanban-tauri/src/index.html`
**Impact:** Non-owner members who join a space have no way to remove it from their sidebar.

**Fix:**
- Add `leave_space_cmd` Tauri command: validate caller is NOT the owner, then call `delete_space()` (same SQL cleanup as owner delete).
- In `renderMembersTab()` in `index.html`: show "Leave Space" button for non-owners (symmetric with "Delete Space" for owner).

---

## HIGH (Fix in next sprint — affects UX or correctness)

### H1. Search Event Listener Memory Leak
**File:** `crates/kanban-tauri/src/index.html:3859-3872`
**Impact:** Listeners accumulate with every search query; DOM nodes are never fully GC'd.

```javascript
container.innerHTML = '';  // Clears DOM but not event listeners
results.forEach((r, i) => {
    const item = document.createElement('div');
    item.addEventListener('click', () => { ... });  // Leaks on re-render
    container.appendChild(item);
});
```

**Fix:** Use event delegation — attach one `click` listener on `#search-results` and read `dataset.*` from the clicked item, removing the per-item listener entirely.

---

### H2. Global Timers Never Cleaned Up
**File:** `crates/kanban-tauri/src/index.html:3788, 3800, 3820`
**Impact:** `_syncStatusInterval` runs every 4 seconds forever. The `_updateCheckTimer` setTimeout on line 3820 overwrites the one on line 3800 without clearing it first, leaking a timer.

**Fix:**
- On line 3820: `clearTimeout(window._updateCheckTimer)` before reassigning.
- Add `window.addEventListener('beforeunload', () => { clearInterval(window._syncStatusInterval); clearTimeout(window._updateCheckTimer); })`.

---

### H3. Missing DB Index: `idx_space_boards_space_id`
**File:** `crates/kanban-storage/src/schema.rs`
**Impact:** `SELECT board_id FROM space_boards WHERE space_id = ?` does a full table scan. Called every time the space main view loads.

**Fix:** Add to v5 migration:
```sql
CREATE INDEX IF NOT EXISTS idx_space_boards_space_id ON space_boards (space_id);
CREATE INDEX IF NOT EXISTS idx_space_members_space_kicked ON space_members (space_id, kicked);
```

---

### H4. alert()/confirm() Used in 20+ Places Instead of Custom UI
**File:** `crates/kanban-tauri/src/index.html` (numerous)
**Impact:** Native dialogs block the event loop, can't be styled, and are inaccessible.

**Fix:** Add a reusable `showConfirm(message, onConfirm)` modal component. Replace all `confirm()` and `alert()` calls with it.

---

### H5. Board/Column Rename Missing
**Files:** `crates/kanban-tauri/src-tauri/src/main.rs`, `crates/kanban-tauri/src/index.html`
**Impact:** Users cannot rename boards or columns after creation.

**Fix:**
- Add `rename_board_cmd(board_id, new_title)` — calls `kanban_core::board::set_board_title()` + updates `cached_title` column (see C1).
- Add `rename_column_cmd(board_id, column_id, new_name)` — calls automerge mutate + syncs.
- In UI: make board title and column headers double-click-to-edit inline.

---

### H6. No Loading/Skeleton States
**File:** `crates/kanban-tauri/src/index.html`
**Impact:** Board loads feel slow; no visual feedback during network operations.

**Fix:** Add `showLoading(container)` / `hideLoading(container)` helpers. Apply on `get_board_detail`, `get_space_cmd`, and other async loads.

---

### H7. New Member Space Doc Sync Incomplete
**File:** `crates/kanban-net/src/swarm.rs`
**Impact:** When a new member joins mid-session, existing members don't re-sync the space doc. New member may not see boards created after join.

**Fix:** In `handle_announced_spaces()`: when a space is already known and a peer is newly connected, send a fresh `Hello` to that peer with the current space doc and board list.

---

### H8. Multiple Hello Messages on Reconnect
**File:** `crates/kanban-net/src/swarm.rs`
**Impact:** On reconnect, multiple Hello handshakes can be sent to the same peer, causing redundant sync and potential state corruption.

**Fix:** Track `hello_sent_peers: HashSet<PeerId>` in swarm state. Skip Hello if already sent and peer is still in `known_peers`.

---

## MEDIUM (Improve when touching related code)

### M1. Excessive Polling Cascade (3 intervals)
**File:** `crates/kanban-tauri/src/index.html:3788, 3210, ~3370`
Sync status polls every 4s, net panel every 5s, chat polls at unknown interval.
When 3+ spaces are expanded, each poll regenerates the sidebar 3+ times.

**Fix:**
- Consolidate: sync status + net panel update into a single 5s interval.
- Move board refresh to event-driven: listen for `board-synced` Tauri event instead of polling.
- Cap chat polling to 10s when no space is focused.

---

### M2. Unused Compiler Warnings (4 active)
- `crates/kanban-core/src/board.rs:1` — unused imports `ObjType`, `ReadDoc`
- `crates/kanban-net/src/swarm.rs:581` — unused import `ReadDoc`
- `crates/kanban-crypto/src/lib.rs:154` — unused variable `sig_bytes`
- `crates/kanban-tauri/src-tauri/src/main.rs:1025` — unused variable `action_tag`

**Fix:** Remove unused imports; prefix unused vars with `_`.

---

### M3. Space Name Update Missing
**Files:** `crates/kanban-tauri/src-tauri/src/main.rs`, `crates/kanban-tauri/src/index.html`
Space names are set at creation but cannot be edited afterwards.

**Fix:** Add `rename_space_cmd(space_id, new_name)` + input field in Space Settings tab.

---

### M4. No Card Filtering UI
**File:** `crates/kanban-tauri/src/index.html`
Global search exists but no per-board filter (by assignee, label, priority).

**Fix:** Add a filter bar above the board columns with dropdowns for assignee/label/priority. Filter client-side by toggling card visibility.

---

### M5. No Undo/Redo Feedback to User
**File:** `crates/kanban-tauri/src/index.html`
Undo (Cmd+Z) and Redo (Cmd+Shift+Z) work silently with no visual confirmation.

**Fix:** Show a brief toast notification ("Undone" / "Redone") using existing `showToast()` helper (or add one).

---

### M6. main.rs Modularity (2219 lines)
**File:** `crates/kanban-tauri/src-tauri/src/main.rs`
All 40+ Tauri commands, view structs, state types, and network integration are in a single file.

**Fix:** Split into:
- `src/views.rs` — all `*View` structs and `*Summary` DTOs
- `src/commands/boards.rs` — board/column/card commands
- `src/commands/spaces.rs` — space/member commands
- `src/commands/profile.rs` — profile, avatar, chat commands
- `src/net_events.rs` — event_rx drain + Tauri event emission
- `src/main.rs` — just `main()`, state setup, command registration

---

### M7. index.html State Management (4000 lines)
**File:** `crates/kanban-tauri/src/index.html`
Global mutable state (`currentSpaceId`, `currentBoardId`, `chatMessages`) scattered throughout 4000-line file.

**Fix:** Extract JS modules (inline `<script type="module">`):
- `state.js` — centralized state with simple pub/sub
- `search.js` — search with event delegation (see H1)
- `sync.js` — consolidated polling lifecycle with cleanup
- `chat.js` — chat polling with timer lifecycle

---

### M8. Persistent Peer Cache Missing
**File:** `crates/kanban-net/src/swarm.rs`
Known peers are loaded at startup from saved peers but not persisted after new peers are discovered mid-session.

**Fix:** On `ConnectionEstablished`, append new peer addresses to the saved peers file so they persist across restarts.

---

### M9. No Reconnection on Network Change
**File:** `crates/kanban-net/src/swarm.rs`
If the user's network interface changes (e.g., VPN, Wi-Fi switch), existing connections drop and new ones are not attempted.

**Fix:** Listen for `libp2p::swarm::SwarmEvent::NewListenAddr` / listen addr changes; re-dial known peers when listen addresses change.

---

## LOW (Polish / nice to have)

### L1. No Notification System
No in-app notifications for mentions, assignments, or new cards from peers.
**Fix:** Add `NotificationView` and a `get_notifications_cmd` that reads from a `notifications` table. Show badge count in sidebar.

### L2. No Data Export
No way to export boards/spaces to JSON or CSV.
**Fix:** Add `export_board_cmd(board_id) -> String (JSON)` + "Export" button in board settings.

### L3. No ARIA Labels on Interactive Elements
Buttons and icons lack `aria-label`. Screen readers cannot describe them.
**Fix:** Add `aria-label` to all icon buttons (delete, edit, close, drag handle).

### L4. XSS Risk in `confirm()` Dialog Text
Some `confirm()` messages interpolate user-controlled strings without escaping.
**Fix:** When replacing confirm() with custom modal (see H4), ensure message text uses `escapeHtml()`.

### L5. No Automerge Doc Compaction
Automerge docs grow indefinitely as changes accumulate. No periodic `compact()` call.
**Fix:** After every 50 mutations (tracked per-board), call `doc.save()` and overwrite the stored bytes to compact history. Or compact on board close.

### L6. Improve Public API Documentation
`kanban-core` and `kanban-storage` have <5 doc comments each.
**Fix:** Add `///` doc comments to all `pub` functions in these crates.

### L7. No Chat Reactions or Threads
Chat is flat messages only; no emoji reactions, no threaded replies.
**Fix (future):** Extend `ChatMessage` struct with `reactions: HashMap<String, Vec<String>>` (emoji → pubkey list) and `reply_to: Option<String>`.

---

## Priority Summary

| ID | Title | Priority | Effort |
|----|-------|----------|--------|
| C1 | N+1 query in list_boards | CRITICAL | Medium |
| C2 | O(n²) history scans in get_board_detail | CRITICAL | Small |
| C3 | get_sync_status_cmd stub | CRITICAL | Small |
| C4 | Leave Space for non-owners | CRITICAL | Small |
| H1 | Search event listener leak | HIGH | Small |
| H2 | Global timers never cleared | HIGH | Small |
| H3 | Missing DB indexes (space_boards, space_members) | HIGH | Small |
| H4 | Replace alert()/confirm() with custom modal | HIGH | Medium |
| H5 | Board/column rename | HIGH | Medium |
| H6 | Loading/skeleton states | HIGH | Medium |
| H7 | New member space doc sync | HIGH | Medium |
| H8 | Multiple Hello on reconnect | HIGH | Small |
| M1 | Polling cascade consolidation | MEDIUM | Small |
| M2 | Fix compiler warnings | MEDIUM | Small |
| M3 | Space name update | MEDIUM | Small |
| M4 | Card filtering UI | MEDIUM | Medium |
| M5 | Undo/redo toast feedback | MEDIUM | Small |
| M6 | main.rs split into modules | MEDIUM | Large |
| M7 | index.html JS modularization | MEDIUM | Large |
| M8 | Persistent peer cache | MEDIUM | Small |
| M9 | Reconnect on network change | MEDIUM | Medium |
| L1 | Notification system | LOW | Large |
| L2 | Data export | LOW | Small |
| L3 | ARIA labels | LOW | Small |
| L4 | XSS in confirm() text | LOW | Small |
| L5 | Automerge doc compaction | LOW | Medium |
| L6 | API documentation | LOW | Medium |
| L7 | Chat reactions/threads | LOW | Large |

---

*Generated 2026-03-29 from 5-agent swarm audit of monotask master post v1-improvements.*
