# Rich Identity + P2P Chat — Design Spec

**Date:** 2026-03-26
**Status:** Approved by user
**Project:** Monotask (Tauri + libp2p + Automerge CRDT)

---

## Overview

Two interconnected features:

1. **Rich Identity** — expand the user profile (photo, bio, role, accent color, presence) and propagate that identity throughout the app UI, replacing raw peer IDs / hex pubkeys with human-readable names and avatars everywhere.
2. **P2P Live Chat** — per-space persistent chat (stored in a dedicated automerge doc, synced like board docs), floating panel UI with `@mention` and `#ref` autocomplete for people, cards, and boards.

---

## 1. Rich Identity

### 1.1 Data Model Changes

**`UserProfile`** (local, in SQLite `profiles` table) — add columns:
- `bio TEXT` — short status/bio string
- `role TEXT` — job title or role (e.g. "Designer", "Lead Dev")
- `color_accent TEXT` — hex color string (e.g. `#c8962a`), chosen by user
- `presence TEXT` — one of `online | away | dnd`; default `online`

**`MemberProfile`** (embedded in SpaceDoc automerge) — add fields:
- `bio: String`
- `role: String`
- `color_accent: String`
- `presence: String`

**`UserProfileView`** (Tauri → frontend) — extend to include all new fields.

**`Member`** (returned from `list_members`) — extend with `bio`, `role`, `color_accent`, `presence`.

### 1.2 Avatar Upload

Two entry points in the profile modal:
- **📁 File** button: opens a native file picker (`dialog::open` Tauri plugin), accepts image files, reads bytes, converts to PNG via `image` crate (resize to max 256×256), stores as `avatar_blob` in DB.
- **📷 Camera** button: opens an in-page `<video>` preview using `getUserMedia({ video: true })`, a "Capture" button grabs a `<canvas>` frame, converts to base64 PNG, sends to `update_my_profile`.

Both paths produce a base64-encoded PNG that gets stored and propagated the same way the current `avatarB64` field does.

### 1.3 Profile Propagation

`update_my_profile` already syncs to all space docs via `MemberProfile`. Extend it to write the four new fields into the automerge map entry for each space the user belongs to. When the space doc is synced to peers, they receive the updated profile automatically.

### 1.4 Identity Everywhere in UI

Replace raw IDs with rich identity chips in:

| Location | Before | After |
|----------|--------|-------|
| Network & Sync panel "Connected Peers" | Raw `12D3KooW…` peer ID chip | Avatar + name + role + presence dot |
| Space members list | Initial letter + truncated pubkey | Avatar image (or colored initial) + name + role badge |
| Card assignee chips | Colored initial | Avatar image + name tooltip |
| Chat message author | — (new) | Avatar + name in accent color |

The bridge between libp2p `PeerId` and `pubkey_hex` is built during Hello handshakes. The swarm already receives the sender's pubkey in the Hello message; store it in a new `peer_pubkeys: HashMap<PeerId, String>` map in swarm state. Expose this via `get_sync_info_cmd` — return `peer_profiles: Vec<PeerIdentity>` alongside `connected_peers`.

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

To resolve profiles: after receiving a peer's pubkey from Hello, look it up in the local space member list (from storage) and populate the profile fields.

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

**Approach:** Separate automerge doc per space, keyed `{space_id}-chat`. Stored in SQLite alongside board docs (in the same `boards` table under a special title `__chat__`, or a new `chat_docs` table). Synced via the existing `SyncProtocol` — the chat doc ID is added to the space's board refs so peers discover and sync it automatically.

**Why this approach:** Reuses the entire sync pipeline with zero new protocol code. Offline peers catch up on reconnect. Consistent with how boards work.

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

New message appended to the end of the list (automerge list insert at `len`).

### 2.3 Backend — New Tauri Commands

| Command | Signature | Description |
|---------|-----------|-------------|
| `send_chat_message_cmd` | `(space_id, text, refs: Vec<ChatRef>)` | Append message to chat doc, trigger sync |
| `get_chat_messages_cmd` | `(space_id, limit: u32, before_ts: Option<u64>)` | Return messages newest-first, paginated |
| `get_mention_suggestions_cmd` | `(space_id, query, kind: "all"\|"member"\|"card"\|"board")` | Autocomplete — search members by name, cards by title, boards by title |

`send_chat_message_cmd` calls `trigger_board_sync(chat_doc_id)` after writing, the same way board edits trigger sync today.

### 2.4 Chat Doc Lifecycle

- **Created** when the space owner first opens chat (or on space creation). Owner writes the initial empty doc and adds `{space_id}-chat` to the space's board refs.
- **Discovered** by peers via the space doc's board refs list — same path as new boards.
- **Synced** — same sync protocol as boards; no special handling needed.
- **Storage key** — use board ID `{space_id}-chat` with a reserved prefix so the UI can filter it out of the boards grid.

### 2.5 Floating Chat Panel UI

**Trigger:** "💬 Chat" button in sidebar footer, positioned above the existing "📶 Network & Sync" button. Only enabled when a space is selected. Shows unread badge count.

**Panel:** Fixed position `bottom: 16px; right: 16px`, width 280px, max-height 420px. Dark teal border (`#1a5a4a`) to distinguish from other modals.

**Sections:**
- **Header** — space name, stacked presence dots for online members, close button
- **Messages list** — scrollable, newest at bottom; each message: circular avatar (photo or colored initial), author name in accent color, timestamp relative (e.g. "2m ago"), message text with inline `@mention` chips (accent colored) and `#ref` chips (teal), clickable
- **Input bar** — single-line text input; `@` triggers member autocomplete, `#` triggers card/board autocomplete; Enter sends; Shift+Enter newline

**Autocomplete dropdown** (appears above input):
- Grouped by type: People / Cards / Boards
- Member row: avatar + name + role
- Card row: card title + column name
- Board row: board name

**Clicking a `#ref` in a message:** navigates to that card or board within the app.

**Clicking an `@mention`:** opens that user's profile card (read-only hover overlay).

### 2.6 Presence

Presence (`online | away | dnd`) is stored in the user's profile and propagated via the space doc on `update_my_profile`. It is **not** derived from connection state — it is a manual user setting. The Network & Sync panel and chat header show the user's stated presence, not connection liveness. Connection liveness (green dot = actually connected right now) is a separate indicator derived from `connected_peers`.

---

## 3. Implementation Sequence

These can be built in two independent work units:

### Unit A — Rich Identity
1. Extend DB schema (`bio`, `role`, `color_accent`, `presence` columns)
2. Extend `UserProfile`, `MemberProfile`, `Member` structs + automerge field writes
3. `update_my_profile` command handles new fields
4. Avatar upload: file picker path + camera capture path in UI
5. `get_sync_info_cmd` returns `peer_profiles` (pubkey→profile lookup via `peer_pubkeys` map in swarm)
6. UI: update profile modal, update peer chips in Network panel, update member list, update card assignee chips

### Unit B — Chat (depends on Unit A for identity rendering)
1. Chat doc storage (`chat_docs` table or boards-table sentinel)
2. `kanban-core`: `ChatMessage` struct + automerge helpers (append, list, paginate)
3. Three new Tauri commands (`send_chat_message_cmd`, `get_chat_messages_cmd`, `get_mention_suggestions_cmd`)
4. Chat doc sync: add `{space_id}-chat` to space board refs on first open
5. UI: floating panel HTML/CSS, message rendering, autocomplete dropdown, `#ref` click navigation

---

## 4. Out of Scope

- Read receipts / message reactions
- Message editing or deletion
- File/image attachments in chat
- Direct messages (DMs) between peers — chat is space-scoped only
- Push notifications
- Chat history export
