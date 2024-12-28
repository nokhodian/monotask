# Rich Identity + P2P Chat — Design Spec

**Date:** 2026-03-26
**Status:** Approved by user
**Project:** Monotask (Tauri v2 + libp2p + Automerge CRDT)

---

## Overview

Two interconnected features:

1. **Rich Identity** — expand the user profile (photo, bio, role, accent color, presence) and propagate that identity throughout the app UI, replacing raw peer IDs / hex pubkeys with human-readable names and avatars everywhere.
2. **P2P Live Chat** — per-space persistent chat (stored in a dedicated automerge doc, synced like board docs), floating panel UI with `@mention` and `#ref` autocomplete for people, cards, and boards.

---

## 1. Rich Identity

### 1.1 Data Model Changes

**`UserProfile`** (local, in SQLite `profiles` table) — add four columns via migration:
```sql
ALTER TABLE profiles ADD COLUMN bio TEXT;
ALTER TABLE profiles ADD COLUMN role TEXT;
ALTER TABLE profiles ADD COLUMN color_accent TEXT;
ALTER TABLE profiles ADD COLUMN presence TEXT DEFAULT 'online';
```

**`MemberProfile`** (embedded in SpaceDoc automerge) — add fields:
- `bio: String`
- `role: String`
- `color_accent: String`
- `presence: String`

**`add_member`** in `kanban-core/src/space.rs` must write all four new fields into the automerge map entry (same `doc.put(&entry, "field", value)` pattern). **`list_members`** must read them back and populate them on `Member` structs, using `.unwrap_or_default()` for missing fields so old space docs without these keys continue to work.

**`UserProfileView`** (Tauri → frontend) — extend to include all new fields.

**`Member`** — extend with `bio: Option<String>`, `role: Option<String>`, `color_accent: Option<String>`, `presence: Option<String>`.

### 1.2 Avatar Upload

Two entry points in the profile modal:
- **📁 File** button: opens a native file picker (`dialog::open` Tauri plugin), accepts image files, reads bytes, converts to PNG via `image` crate (resize to max 256×256), stores as `avatar_blob` in DB.
- **📷 Camera** button: opens an in-page `<video>` preview using `getUserMedia({ video: true })`, a "Capture" button grabs a `<canvas>` frame, converts to base64 PNG, sends to `update_my_profile`.

Both paths produce a base64-encoded PNG stored and propagated the same way the current `avatarB64` field does.

### 1.3 Profile Propagation

`update_my_profile` already syncs to all space docs via `MemberProfile`. Extend it to write the four new fields into the automerge map entry for each space the user belongs to. When the space doc syncs to peers, they receive the updated profile automatically.

### 1.4 Identity Everywhere in UI

Replace raw IDs with rich identity chips in:

| Location | Before | After |
|----------|--------|-------|
| Network & Sync panel "Connected Peers" | Raw `12D3KooW…` peer ID chip | Avatar + name + role + presence dot |
| Space members list | Initial letter + truncated pubkey | Avatar image (or colored initial) + name + role badge |
| Card assignee chips | Colored initial | Avatar image + name tooltip |
| Chat message author | — (new) | Avatar + name in accent color |

**Bridging libp2p `PeerId` → `pubkey_hex`:**

The `Hello` message struct does **not** contain a sender pubkey field. The sender's public key is already available via the `pubkey_cache: HashMap<PeerId, libp2p::identity::PublicKey>` maintained by the Identify protocol handler in `swarm.rs`. This cache is populated before Hello is processed.

To get the hex ed25519 pubkey from a cached `libp2p::identity::PublicKey`:
```rust
let hex_pubkey = key
    .try_into_ed25519()           // extract ed25519 key
    .map(|k| hex::encode(k.to_bytes()))  // hex-encode the 32 raw bytes
    .ok();
```

On each successful Hello exchange, the swarm stores `peer_id → hex_pubkey` in a new `peer_pubkeys: HashMap<PeerId, String>` map (derived from `pubkey_cache` at Hello time). This map lives inside the swarm's async loop.

A new `NetCommand::GetPeerPubkeys { reply: oneshot::Sender<HashMap<String, String>> }` variant exposes it externally. The swarm handler responds with a clone of the map (both keys and values as plain strings). A corresponding `get_peer_pubkeys_sync()` method is added to `NetworkHandle`.

`get_sync_info_cmd` calls `get_peer_pubkeys_sync()`, then for each connected peer looks up the pubkey in the local space member list (via `list_members`), and returns `peer_profiles: Vec<PeerIdentity>`.

```rust
struct PeerIdentity {
    peer_id: String,
    pubkey: String,
    display_name: Option<String>,
    role: Option<String>,
    color_accent: Option<String>,
    presence: Option<String>,
    avatar_b64: Option<String>,
}
```

### 1.5 Profile Modal UI

Sections (top to bottom):
1. **Avatar** — 72px circular photo with colored ring matching accent color; "📁 File" + "📷 Camera" buttons below
2. **Display Name** + **Role / Title** — side by side in the right column
3. **Status / Bio** — full-width textarea (max 120 chars)
4. **Accent Color** — 6 preset swatches + a custom color picker (`<input type="color">`)
5. **Presence** — radio buttons: Online (green) / Away (amber) / Do Not Disturb (grey)
6. **Public Key** — read-only monospace with copy button (existing)
7. **SSH Key Path** — existing field, unchanged

---

## 2. P2P Chat

### 2.1 Architecture

**Approach:** Separate automerge doc per space, stored in the existing `boards` SQLite table with a new `is_system BOOLEAN DEFAULT 0` column. The chat doc row has `is_system = 1` so the UI filters it from the boards grid. Synced via the existing `SyncProtocol` — the chat doc ID is added to the space's board refs so peers discover and sync it automatically.

**Why this approach:** Reuses the entire sync pipeline with zero new protocol code. Offline peers catch up on reconnect. Consistent with how boards work.

**Sync scope:** `trigger_board_sync(chat_doc_id)` syncs with all currently connected peers — the same behaviour as boards. Only peers who share the space will have the chat doc in their board refs and will accept the sync.

### 2.2 Chat Message Data Model

Each message is an entry in an automerge `List` at the root of the chat doc:

```
ROOT
  messages: List<Map>
    [i]:
      id: String          — UUID
      author: String      — pubkey hex
      text: String        — raw message text
      created_at: u64     — unix seconds
      refs: List<Map>     — inline references
        [j]:
          kind: String    — "card" | "board" | "member"
          id: String      — entity ID
          label: String   — display text at time of send
```

New messages are appended to the end of the list (automerge list insert at `len`).

### 2.3 Backend — New Tauri Commands

| Command | Signature | Description |
|---------|-----------|-------------|
| `send_chat_message_cmd` | `(space_id, text, refs: Vec<ChatRef>)` | Append message to chat doc, trigger sync |
| `get_chat_messages_cmd` | `(space_id, limit: u32, before_ts: Option<u64>)` | Return messages newest-first, paginated |
| `get_mention_suggestions_cmd` | `(space_id, query, kind: "all"\|"member"\|"card"\|"board")` | Autocomplete results |

`send_chat_message_cmd` calls `trigger_board_sync(chat_doc_id)` after writing.

**`get_mention_suggestions_cmd` search scope:**
- **Members:** call `list_members` on the space doc (in memory, cheap).
- **Boards:** query the `boards` SQLite table `WHERE space_id = ? AND is_system = 0` — board titles are a `title` column there. No automerge load needed.
- **Cards:** card titles live inside individual automerge board docs and are not indexed in SQLite. Add a `card_search_index` table:
  ```sql
  CREATE TABLE card_search_index (
    card_id TEXT PRIMARY KEY,
    board_id TEXT NOT NULL,
    space_id TEXT NOT NULL,
    title TEXT NOT NULL,
    column_name TEXT NOT NULL
  );
  ```
  This table is updated (INSERT OR REPLACE / DELETE) in the Tauri command handlers for `create_card_cmd`, `update_card_cmd`, and `delete_card_cmd`. `get_mention_suggestions_cmd` queries `WHERE space_id = ? AND title LIKE '%query%'`.

### 2.4 Chat Doc Lifecycle

**Deterministic ID:** The chat doc ID is always the fixed string `{space_id}-chat`. This ensures any peer that bootstraps the doc independently creates the same ID. When two peers create docs with the same ID before syncing, the automerge sync protocol merges them without data loss (concurrent inserts into the messages list both survive).

- **Bootstrap:** When any peer opens the chat panel for a space and no chat doc exists locally, they create an empty automerge doc, save it to the `boards` table with `id = '{space_id}-chat'`, `is_system = 1`, and call `add_board_ref(space_doc, '{space_id}-chat')` to register it in the space doc. The updated space doc propagates to peers on next sync, who then discover and pull the chat doc.
- **Empty state:** If no chat doc exists and the peer has not yet synced, the panel shows "No messages yet — send one to start the conversation."
- **Boards grid filter:** The UI filters boards where `is_system = true` from the boards grid display. This is a semantic flag, not a suffix check, so it works regardless of ID format.

### 2.5 Floating Chat Panel UI

**Trigger:** "💬 Chat" button in sidebar footer, positioned above the existing "📶 Network & Sync" button. Disabled (greyed out) when no space is selected. Shows an unread message count badge when the panel is closed and new messages arrive.

**Panel:** Fixed position `bottom: 16px; right: 16px`, width 280px, max-height 420px. Dark teal border (`#1a5a4a`). `z-index: 300` (above Network panel at 200).

**Sections:**
- **Header** — space name, stacked presence dots for online members, close button
- **Messages list** — scrollable, newest at bottom; each message: circular avatar (photo or colored initial), author name in accent color, timestamp relative (e.g. "2m ago"), message text with inline `@mention` chips (accent colored) and `#ref` chips (teal), clickable
- **Input bar** — single-line text input; `@` triggers member autocomplete, `#` triggers card/board autocomplete; Enter sends; Shift+Enter newline

**Autocomplete dropdown** (appears above input):
- Grouped by type: People / Cards / Boards
- Member row: avatar + name + role
- Card row: card title + column name
- Board row: board name

**Clicking a `#ref` in a message:** navigates to that card or board.

**Clicking an `@mention`:** opens that user's profile card (read-only hover overlay).

### 2.6 Presence

Presence (`online | away | dnd`) is a manual user setting stored in the profile, propagated via the space doc. Connection liveness (peer is actively connected right now) is a separate indicator derived from `connected_peers` — shown as a distinct "live" dot independent of the presence field.

---

## 3. Implementation Sequence

### Unit A — Rich Identity
1. DB migration: add `bio`, `role`, `color_accent`, `presence` to `profiles`; add `is_system` to `boards`
2. Extend `MemberProfile` struct (kanban-core); update `add_member` to write new fields; update `list_members` to read them with `unwrap_or_default()` fallback
3. Extend `UserProfile`, `UserProfileView`, `Member` structs; update `get_my_profile` and `update_my_profile` Tauri commands
4. Add `NetCommand::GetPeerPubkeys` + swarm handler (derive hex pubkey from `pubkey_cache` via `try_into_ed25519().to_bytes()`); add `get_peer_pubkeys_sync()` to `NetworkHandle`
5. Extend `get_sync_info_cmd` to call `get_peer_pubkeys_sync()`, cross-reference with `list_members`, return `peer_profiles: Vec<PeerIdentity>`
6. Avatar upload: file picker Tauri command + camera `getUserMedia` UI flow
7. UI: profile modal redesign; identity chips in Network panel, member list, card assignees

### Unit B — Chat (depends on Unit A for identity rendering)
1. Add `card_search_index` SQLite table; populate in `create_card_cmd`, `update_card_cmd`, `delete_card_cmd`
2. `kanban-core`: `ChatMessage` struct + automerge helpers (append, list, paginate)
3. Three new Tauri commands: `send_chat_message_cmd`, `get_chat_messages_cmd`, `get_mention_suggestions_cmd`
4. Chat doc bootstrap: create on first open with deterministic ID `{space_id}-chat`, `is_system = 1`, add to space board refs; filter `is_system` boards from boards grid
5. UI: floating panel HTML/CSS, message rendering with identity, autocomplete dropdown, `#ref` click navigation, unread badge

---

## 4. Out of Scope

- Read receipts / message reactions
- Message editing or deletion
- File/image attachments in chat
- Direct messages (DMs) between peers — chat is space-scoped only
- Push notifications
- Chat history export
