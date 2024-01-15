# P2P Kanban — Feature Additions Design Spec
**Date:** 2026-03-18
**Status:** Approved
**Supplements:** `P2P_Kanban_Execution_Plan_v2.md`

---

## Overview

This document specifies eight feature additions to the P2P Kanban v2.0 plan. Each addition is scoped as a patch to the relevant existing module. No existing architectural decisions are overturned. All additions follow the same patterns established in the base plan: Automerge CRDTs for shared state, SQLite for local persistence, Iroh for P2P transport, Tauri commands + channels for GUI/backend IPC.

**Additions covered:**

1. Undo/Redo (local user scope)
2. @Mentions
3. Card Linking + Human-readable card numbers
4. Missing CLI commands (comments, checklists)
5. Card copy/duplicate
6. Protocol version negotiation
7. Peer presence indicators
8. Deep link URL scheme (`kanban://`)

---

## 1. Undo/Redo

**Modules affected:** `kanban-core`, `kanban-storage`, `kanban-cli`, `kanban-tauri`

### Design

Undo/redo is **local-only** — it applies only to the acting user's own operations on their own machine. It never propagates to peers as an "undo" concept. This avoids Byzantine complexity: a peer cannot undo another peer's action.

The mechanism is a compensating-operation stack. Automerge does not expose a first-class inverse-change API, so `kanban-core` constructs the compensating operation semantically at the domain level, not at the Automerge byte level. For example:

- `move_card(card_id, from_col, to_col, from_pos, to_pos)` → inverse = `move_card(card_id, to_col, from_col, to_pos, from_pos)`
- `rename_card(card_id, old_title, new_title)` → inverse = `rename_card(card_id, new_title, old_title)`
- `delete_card(card_id, snapshot)` → inverse = `restore_card(card_id, snapshot)` (snapshot captured at delete time)

The inverse is stored as a CBOR-encoded domain-level operation struct, not as raw Automerge change bytes. On undo, `kanban-core` re-executes the inverse as a new `kanban-core` operation, which generates a fresh signed Automerge change. Peers receive it like any other operation and converge correctly.

**Tombstone guard:** Before applying an inverse, `kanban-core` checks whether the target object (card, column, comment) still exists in the Automerge document. If the object has been tombstoned or deleted by a peer in the interim, the undo is aborted — the inverse is popped from the stack but not applied. The GUI shows an error toast: *"Cannot undo — this item was deleted by another peer."* This prevents silent no-op or panics against deleted Automerge objects.

### Storage

`kanban-storage` adds `undo_stack` and `redo_stack` tables to SQLite (local database only, never synced to peers):

```sql
CREATE TABLE undo_stack (
    board_id    TEXT NOT NULL,
    actor_key   TEXT NOT NULL,       -- public key of the acting user
    seq         INTEGER NOT NULL,    -- monotonically increasing per (board, actor)
    action_tag  TEXT NOT NULL,       -- e.g. "move_card", "delete_card", "rename_column"
    inverse_op  BLOB NOT NULL,       -- CBOR-encoded domain-level compensating operation
    hlc         TEXT NOT NULL,
    PRIMARY KEY (board_id, actor_key, seq)
    -- Access pattern: SELECT ... WHERE board_id=? AND actor_key=? ORDER BY seq DESC LIMIT 1
    -- The composite PK index covers this query. Depth pruning (DELETE WHERE seq < cutoff)
    -- also uses the PK prefix scan; no additional index is needed.
);

CREATE TABLE redo_stack (
    board_id    TEXT NOT NULL,
    actor_key   TEXT NOT NULL,
    seq         INTEGER NOT NULL,
    action_tag  TEXT NOT NULL,
    forward_op  BLOB NOT NULL,       -- CBOR-encoded domain-level forward operation
    hlc         TEXT NOT NULL,
    PRIMARY KEY (board_id, actor_key, seq)
);
```

### Constraints

- **Default depth:** 50 operations per user per board. Configurable via `undo_depth` in `~/.config/p2p-kanban/config.toml`. Enforced at write time: after pushing to `undo_stack`, delete rows where `seq < (MAX(seq) - undo_depth)` for the same `(board_id, actor_key)`.
- **Post-compaction:** Operations older than the last `prune` are not undoable. `kanban-core` clears both stacks for a board when `prune` is run.
- **Tombstone guard:** If the target object no longer exists, undo/redo is aborted. The failed operation is **popped and discarded** from the stack (not left in place). The GUI shows an error toast: *"Cannot undo — this item was deleted by another peer."* The remaining stack entries are preserved and usable.
- **Peer-modified objects:** If the object exists but was modified by a peer, the inverse applies via CRDT merge. The result may be unexpected. The GUI shows a warning toast: *"Undo applied — note that [Alice] also modified this item."*
- **Redo invalidation by local action:** Any new local user action after an undo clears the redo stack (standard undo/redo semantics).
- **Redo invalidation by peer tombstone:** When a redo operation is aborted due to a peer having deleted the target object, that single redo entry is popped and discarded. The user is notified via error toast. The remaining redo stack is unaffected. There is no automatic full-stack clear on peer activity — only the specific poisoned entry is removed.

### CLI additions (Module 5 patch)

```
app-cli undo <board_id> [--json]               # Apply top of undo stack
app-cli redo <board_id> [--json]               # Apply top of redo stack
app-cli undo-history <board_id> [--json]       # List local undo stack (action_tag + hlc)
```

Note: `undo-history` is a distinct top-level command (not a sub-subcommand of `undo`) to avoid clap argument parsing ambiguity where a board named "history" would be misrouted.

**CLI behavior for `copy_card` undo with subsequent modifications:** When `app-cli undo <board_id>` targets a `copy_card` operation and the copied card has been subsequently modified, the command aborts and prints to stderr:

```
Error: undoing this copy would delete card 'a7f3-2' which has been modified since copying.
       Use --force to proceed and discard those changes.
```

With `--force`, the undo applies without prompting.

### GUI additions (Module 7 patch)

- Keyboard shortcuts: Ctrl+Z / Ctrl+Y (Cmd+Z / Cmd+Shift+Z on macOS)
- Success toast on undo/redo: *"Undone: moved card 'Deploy API' to In Progress"*
- Error toast when target is tombstoned: *"Cannot undo — this item was deleted by another peer."*
- Warning toast when undone object was peer-modified (see Constraints above)
- Edit menu items: Undo / Redo with greyed state when stack is empty

---

## 2. @Mentions

**Modules affected:** `kanban-core`, `kanban-storage`, `kanban-cli`, `kanban-tauri`

### Design

Mentions are parsed from card description and comment text as they arrive (both local and peer changes). No separate CRDT structure is required — mentions are implicit in the text. A `mention_index` table is materialized locally by scanning Automerge changes as they arrive.

### Syntax

`@alias` typed in the GUI triggers a search-as-you-type dropdown of board members. Selecting a member inserts a structured token:

```
@[Alice|pk_7xq3m...]
```

- Human-readable in plain text export and CLI output
- Machine-parseable for mention indexing
- Aliases are resolved against `doc.members` (the board member map)
- If two members share an alias, the GUI shows a disambiguation picker
- `@all` notifies every board member; stored as `@[all]` and materialized per-peer as a self-mention

### Storage (local only, never synced)

```sql
CREATE TABLE mention_index (
    board_id     TEXT NOT NULL,
    card_id      TEXT NOT NULL,
    mentioned    TEXT NOT NULL,    -- public key of mentioned user
    mentioned_by TEXT NOT NULL,    -- public key of author
    context      TEXT NOT NULL,    -- "description" | "comment:<comment_id>"
    hlc          TEXT NOT NULL,
    seen         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (board_id, card_id, mentioned, context)
);

-- Index for the common query: fetch unseen mentions for the local user
CREATE INDEX idx_mention_index_unseen ON mention_index (mentioned, seen, hlc DESC);
```

Rebuilt incrementally, not from scratch. `kanban-storage` persists the last-scanned Automerge change hash in a `meta` table:

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- Row: ('mention_scan_checkpoint', '<last_scanned_change_hash>')
```

On startup, `kanban-core` loads the checkpoint and scans only Automerge changes that arrived after it. On a fresh install (no checkpoint), the full log is scanned once and the checkpoint is written. Subsequent restarts are bounded by changes since the last session.

### Notification flow

1. Incoming peer change arrives → `kanban-net` → `kanban-crypto` verifies → `kanban-core` applies merge
2. `kanban-core` scans new text fields in the change for `@[alias|pubkey]` tokens matching the local user's public key
3. Match found → `INSERT OR IGNORE INTO mention_index` with `seen = 0`
4. Tauri Channel emits `MentionEvent { board_id, card_id, mentioned_by, context }`
5. GUI: system tray badge count increments + in-app notification banner + "Mentions" sidebar tab count updates

### Mentions view (GUI)

A dedicated tab in the activity sidebar. Shows unseen mentions first (bold), then recent seen mentions. Each entry displays: author alias, card title, snippet of context text, timestamp. Clicking an entry navigates to the card/comment and marks `seen = 1`.

### CLI additions (Module 5 patch)

```
app-cli mentions list [--board <id>] [--unread] [--json]    # List mentions for local user
app-cli mentions mark-read [--board <id>] [--json]           # Mark all mentions as seen
```

---

## 3. Card Linking + Human-Readable Card Numbers

**Modules affected:** `kanban-core`, `kanban-storage`, `kanban-cli`, `kanban-tauri`

### 3a. Human-readable card numbers (prerequisite)

Card numbers are a prerequisite for usable card linking. UUIDs are not suitable for human reference.

Each board maintains a monotonically increasing per-actor counter in the Automerge document:

```
doc.actor_card_seq[pubkey]: Counter   -- per-actor sequence number; incremented at creation
doc.cards[card_id].number: String     -- composite: "<actor_prefix>-<seq>", e.g. "a7f3-1"
```

**Why not a shared PN-Counter for sequential integers:** A shared PN-Counter tracks the total; it does not atomically allocate unique sequential slots per peer. Two peers reading `counter = 41` and each incrementing locally would both assign `#42`, producing duplicate display numbers in the merged document. This is a fundamental correctness issue.

**Chosen approach — actor-scoped sequential numbers:**

- Each user's cards are numbered within their own sequence: `a7f3-1`, `a7f3-2`, etc.
- The actor prefix is the first 4 characters of the base32-encoded public key.
- `doc.actor_card_seq[pubkey]` is a per-actor Automerge Counter. Each actor only writes to their own key, so concurrent increments are conflict-free.
- Display: `#a7f3-42`. CLI and GUI both accept short form (`a7f3-42`) or just the integer suffix when unambiguous within an actor's cards.
- **Prefix collision:** Two members sharing the same 4-character prefix is statistically rare (32^4 = ~1M combinations) but must be handled. On board load, `kanban-core` checks all member public keys for prefix collisions. If a collision is detected, all affected actors' prefixes are extended to 8 characters for that board. The `card_number_index` is rebuilt with the extended prefixes. No card numbers already stored in the Automerge document are mutated — the extended prefix is a display-layer resolution only, derived from the full public key.

**Resolution for existing UUIDs:** Boards that existed before this feature was introduced assign numbers during a one-time migration pass. Each actor runs the migration **only for cards they created** (`doc.cards[id].created_by == local_pubkey`). This is conflict-free: only one peer writes each card's number (the creator), matching the invariant for new cards. Cards whose creator is no longer a board member receive numbers when that creator next runs the app; until then, they display their UUID truncated to 8 characters as a fallback label.

Card numbers are **display-only**. The canonical key remains the UUID. All CLI commands and API surfaces accept both:

```
app-cli card show <board_id> a7f3-42      # resolves to UUID internally
app-cli card show <board_id> <uuid>       # also valid
```

GUI renders card numbers in the card header and all reference contexts.

### 3b. Card linking

Links are stored in the Automerge document using a **Map keyed by card_id** (not a List) to ensure concurrent add operations are idempotent:

```
doc.cards[card_id].related: Map<card_id, true>
```

Using `Map<card_id, bool>` (value always `true`) means:
- Adding the same link from two peers concurrently = both write `related[target_id] = true` → converges to a single entry (Automerge map LWW per key)
- Removing a link = `delete related[target_id]` → concurrent add and remove resolves to the last-write winner per Automerge map semantics

Links are **undirected and board-scoped**. When card A links to card B, `kanban-core` writes to both `doc.cards[A].related[B]` and `doc.cards[B].related[A]` in a single Automerge transaction.

**Split-link repair:** Because the two sides of a link are separate Automerge map entries, concurrent operations (e.g., peer X adds `A→B` while peer Y removes `B→A`) can produce a split state where `A.related[B]` exists but `B.related[A]` does not. `kanban-core` runs a periodic reconciliation pass (at most once every 60 seconds per board, not on every card read) that scans all cards' `related` maps, identifies missing reverse entries, and issues a single batched Automerge transaction to repair all splits found in that pass.

To prevent repair-write storms when multiple peers are online simultaneously, the repair pass is gated by a **jitter delay**: each peer waits a random interval in `[0, 30s]` before running. If a peer's Automerge sync reveals the split has already been repaired by another peer (i.e., the repair change arrives during the jitter window), the local repair is skipped. A `repairing_links` in-memory flag per board prevents re-entrant repair invocations within the same pass. Repair writes are not pushed to the `undo_stack`.

**Cross-board links** are out of scope. A link to a card on a board the local user does not have renders as `[card not found — a7f3-42]` in the GUI rather than throwing an error.

**Copied-from reference:** When a card is duplicated (see Section 5), `doc.cards[new_id].copied_from` stores the source card ID as a read-only provenance string (not part of `related`, not shown as a link in the UI).

### GUI additions

Card detail modal shows a "Related cards" section beneath checklists. Typing `#` in the modal triggers a search-as-you-type card picker (searches by number and title). Each related card renders as `#a7f3-42 — Deploy API` with the target card's column color as a status indicator. Clicking a related card opens its detail modal with a breadcrumb back.

### CLI additions (Module 5 patch)

```
app-cli card link <board_id> <card_id> <target_card_id> [--json]     # Add relation
app-cli card unlink <board_id> <card_id> <target_card_id> [--json]   # Remove relation
app-cli card links <board_id> <card_id> [--json]                      # List related cards
```

---

## 4. Missing CLI Commands (Comments & Checklists)

**Module affected:** `kanban-cli`

The base plan adds comments and checklists to `kanban-core` but omits CLI commands for them. These are required for the CLI to reach parity with the GUI. All mutation commands support `--json` to return the created/updated resource (UUID, card number, HLC) for scripting — consistent with the base plan convention.

### CLI additions (Module 5 patch)

```
# Comments
app-cli card comment add <board_id> <card_id> <text> [--json]
app-cli card comment list <board_id> <card_id> [--json]
app-cli card comment delete <board_id> <card_id> <comment_id> [--json]
# Note: comment editing is out of scope for MVP (base plan decision); delete + re-add is the workaround

# Checklists
app-cli checklist add <board_id> <card_id> <title> [--json]
app-cli checklist rename <board_id> <card_id> <checklist_id> <new_title> [--json]
app-cli checklist delete <board_id> <card_id> <checklist_id> [--json]
app-cli checklist item add <board_id> <card_id> <checklist_id> <text> [--json]
app-cli checklist item check <board_id> <card_id> <checklist_id> <item_id> [--json]
app-cli checklist item uncheck <board_id> <card_id> <checklist_id> <item_id> [--json]
app-cli checklist item delete <board_id> <card_id> <checklist_id> <item_id> [--json]
```

JSON output for mutation commands returns at minimum: `{ "id": "<uuid>", "board_id": "...", "hlc": "..." }`.

---

## 5. Card Copy/Duplicate

**Modules affected:** `kanban-core`, `kanban-storage`, `kanban-cli`, `kanban-tauri`

Note: `kanban-storage` is included because the `copy_card` operation persists an Automerge change (via the standard write path) and also pushes an entry to the `undo_stack`.

### Design

`kanban-core` exposes a `copy_card` operation that generates a new card in a single Automerge transaction. The copy is a standalone card — it is not linked to the original (though `copied_from` records provenance). Like all card creation operations, it is signed and broadcast to peers via Iroh gossip.

**Fields copied:**
- `title` (prefixed with "Copy of " by default, editable immediately in GUI)
- `description`
- `labels`
- `checklists` (items copied with `checked = false` — fresh start)

**Fields cleared on copy:**
- `assignees` — empty (a copy starts unassigned)
- `comments` — empty (conversation does not transfer)
- `related` — empty (links do not transfer)
- `due_date` — cleared

**Fields set fresh:**
- New UUID and actor-scoped card number
- `created_at` = current HLC
- `created_by` = local user's public key
- `copied_from` = source card_id (read-only provenance string, not a link)

**Placement:** The copy is inserted immediately after the source card in the same column by default. `--to-column` overrides the target column; position defaults to end of that column.

### CLI additions (Module 5 patch)

```
app-cli card copy <board_id> <card_id> [--to-column <col_id>] [--json]
```

### GUI additions (Module 7 patch)

Right-click context menu on a card → "Duplicate". The duplicate appears immediately below the original (optimistic UI). The title field opens in edit mode so the user can rename it immediately.

**Undo warning:** If the user undoes a `copy_card` operation and the copied card has been subsequently modified (title changed, items checked, comments added), `kanban-core` detects this by checking whether any changes to the copy's card_id exist in the Automerge log after the original copy transaction. If so, the GUI shows a confirmation dialog before applying the undo: *"Undoing this copy will delete card '#a7f3-2 — Copy of Deploy API' and all changes made to it. Continue?"* Clicking "Undo anyway" proceeds; clicking "Cancel" leaves the undo stack unchanged.

---

## 6. Protocol Version Negotiation

**Module affected:** `kanban-net`

### Design

The risk register (Risk #1) identifies Iroh breaking changes as high-likelihood/high-impact, but the base plan contains no mechanism to handle version mismatches between peers. This addition defines the handshake.

### Version hello message

Immediately after an Iroh QUIC connection is established (before any board data is exchanged), both peers exchange a `VersionHello` CBOR message:

```rust
struct VersionHello {
    app_version:       String,         // semver, e.g. "0.3.1"
    min_compatible:    String,         // semver, e.g. "0.2.0"
    iroh_version:      String,         // e.g. "0.96.0"
    protocol_features: Vec<String>,    // feature flags, e.g. ["undo_v1", "mentions_v1"]
}
```

`min_compatible` is a compile-time constant bumped manually on any breaking change to the Automerge document schema or the wire protocol. Patch versions never bump `min_compatible`.

**Handshake timeout:** Each peer waits a maximum of **5 seconds** for the remote `VersionHello` after the QUIC connection is established. If no `VersionHello` is received within this window (e.g., the remote is running a pre-handshake version that never sends it), the connection is closed and treated as incompatible. The local user sees: *"Peer did not respond to version handshake — they may be running an older incompatible version."* The timeout is configurable via `version_handshake_timeout_secs` in `config.toml`.

### Message types

```rust
struct VersionHello {
    app_version:       String,
    min_compatible:    String,
    iroh_version:      String,
    protocol_features: Vec<String>,
}

struct VersionReject {
    reason:       String,   // "version_too_old" | "version_too_new"
    min_required: String,   // the sender's min_compatible
    their_version: String,  // the rejected peer's app_version (echoed back for clarity)
}
```

Both are CBOR-encoded. `kanban-net` dispatches received messages by inspecting the CBOR tag: `VersionHello` uses tag `0x6B01`, `VersionReject` uses tag `0x6B02`. On receiving a `VersionReject`, `kanban-net` closes the connection and emits a user-visible error: *"Connection rejected by peer: your version {their_version} is below their minimum {min_required}."*

### Compatibility check logic

Version comparison **must use the `semver` crate** (or equivalent proper semver parser) — not lexicographic string comparison. Lexicographic comparison is incorrect for semver: `"0.9.0" > "0.10.0"` under lexicographic order, which is wrong.

```
let remote_ver = semver::Version::parse(&remote.app_version)?;
let local_min  = semver::Version::parse(&local.min_compatible)?;

if remote_ver < local_min:
    → send VersionReject { reason: "version_too_old", min_required: local.min_compatible }
    → close connection
    → display to user: "Peer is running v{remote.app_version}, which is incompatible.
                        Ask them to upgrade to v{local.min_compatible}+."

// symmetric: remote performs the same check on our hello
```

`VersionReject` is sent before closing so the remote peer also displays a human-readable error rather than a raw connection failure.

### Feature flags

`protocol_features` is a list of optional feature identifiers. Peers intersect their feature lists to determine which optional protocols to activate for this session. This allows incremental rollout of new features without requiring all peers to upgrade simultaneously.

Example flags corresponding to this spec: `"undo_v1"`, `"mentions_v1"`, `"presence_v1"`, `"card_numbers_v1"`, `"deep_link_v1"`.

### CLI additions (Module 5 patch)

```
app-cli status [--json]
```

Already exists in the base plan. The `--json` output is extended with:
```json
{
  "protocol_version": "0.3.1",
  "min_compatible": "0.2.0",
  "active_features": ["undo_v1", "mentions_v1"]
}
```

---

## 7. Peer Presence Indicators

**Modules affected:** `kanban-net`, `kanban-core`, `kanban-tauri`

### Design

Presence indicates which peers are currently active and what they are focused on. It is **ephemeral** — never persisted to SQLite, never part of the Automerge document. Presence lives entirely in-memory.

### Protocol

Peers broadcast a `PresenceHeartbeat` via the board's Iroh gossip topic every 5 seconds:

```rust
struct PresenceHeartbeat {
    actor_key:    PublicKey,
    board_id:     String,
    focused_card: Option<String>,   // card_id if a card detail is open, None if board view
    hlc:          HybridTimestamp,
}
```

Heartbeats use a **lightweight signed envelope** distinct from the Automerge change envelope. The Automerge change envelope includes sequence numbers and document references used by the CRDT apply path — applying it to a heartbeat would cause `kanban-core` to attempt an Automerge merge, which must not happen for ephemeral messages.

The heartbeat envelope:

```rust
struct SignedPresence {
    payload:   PresenceHeartbeat,   // CBOR-encoded
    author:    PublicKey,
    signature: Signature,           // signs the CBOR-encoded payload bytes
}
```

`kanban-net` routes messages based on a 1-byte type prefix in the Iroh gossip payload: `0x01` = Automerge change envelope, `0x02` = SignedPresence, `0x03` = VersionHello/Reject. This prevents the Automerge apply path from ever processing presence messages.

TTL = 15 seconds. If no heartbeat is received within 15 seconds, the peer is considered offline/inactive. `kanban-net` maintains a per-board in-memory `HashMap<PublicKey, PresenceHeartbeat>`.

### Security considerations

Heartbeats are signed but **membership is not re-verified** per heartbeat — membership was already verified during the board join handshake (challenge-response per base plan Module 2). Only peers who have already authenticated as board members can participate in the board's Iroh gossip topic. Non-members cannot subscribe to the topic and therefore cannot inject or observe heartbeats. This provides sufficient isolation; no additional per-heartbeat membership check is needed.

Heartbeats do reveal which card IDs are actively viewed. Since all gossip topic participants are verified board members, this is acceptable. Heartbeats from an unexpected public key (e.g., a stale connection from a banned peer) are silently discarded by `kanban-core` after checking `doc.members` — card IDs are not displayed.

### GUI additions (Module 7 patch)

- **Card detail modal:** Avatar dots of peers currently viewing that card appear in the modal header (max 3 shown inline, "+N others" overflow label).
- **Board view:** Cards with at least one active peer presence show a small colored dot in their top-right corner.
- **Peer list panel (existing):** Shows each peer's current focus: *"Viewing #a7f3-42 — Deploy API"* or *"On board view"*.

### Performance

Heartbeats are ~150 bytes each, scoped per board gossip topic. At 50 peers and 5s intervals, this is ~1.5KB/s per board — negligible.

---

## 8. Deep Link URL Scheme

**Modules affected:** `kanban-tauri`, `kanban-cli`

### Design

A custom URL scheme `kanban://` enables external tools — scripts, terminal output, git hooks, IDE extensions — to deep-link into specific boards and cards in the running GUI.

### URL format

```
kanban://board/<board_id>
kanban://card/<board_id>/<card_ref>
```

`<card_ref>` is a single path segment that may be either a UUID or a human-readable card number. **Resolution rule:** try to parse `<card_ref>` as a card number (e.g., `a7f3-42`) first by matching the `<4-char-prefix>-<integer>` pattern. If it matches, resolve by querying a local SQLite card index (see below). If it does not match the pattern, treat it as a UUID and look up directly in the Automerge document. This ensures unambiguous routing — card numbers and UUIDs have structurally distinct formats.

`kanban-storage` maintains a local card number index, kept current by `kanban-core` after every Automerge merge — whether from a local commit or an incoming peer change delivered via gossip. After each merge, `kanban-core` calls `sync_card_number_index(board_id, changed_card_ids)` to upsert only the affected rows. This ensures peer migration writes (card number assignments arriving via gossip) are immediately resolvable without waiting for a restart. On startup the index is rebuilt in full as a bootstrap:

```sql
CREATE TABLE card_number_index (
    board_id   TEXT NOT NULL,
    card_id    TEXT NOT NULL,   -- UUID
    number     TEXT NOT NULL,   -- actor-scoped number, e.g. "a7f3-42"
    PRIMARY KEY (board_id, card_id)
);
CREATE UNIQUE INDEX idx_card_number_lookup ON card_number_index (board_id, number);
```

```
kanban://card/my-board/a7f3-42     # card number → resolved to UUID
kanban://card/my-board/<uuid>      # UUID → direct lookup
```

Registration is handled by Tauri's deep link plugin (`tauri-plugin-deep-link`), which registers the `kanban://` scheme at install time on all three platforms (macOS `Info.plist`, Linux `.desktop` file, Windows registry).

### Behavior

- **App running:** deep link is dispatched to the running instance, which navigates to the target board/card and raises the window.
- **App not running:** app launches and opens directly to the target.
- **Board/card not found locally:** app shows a not-found screen with the option to join the board if the user has an invite token.

### CLI additions (Module 5 patch)

For scripts that need to open the GUI programmatically:

```
app-cli open board <board_id>
app-cli open card <board_id> <card_ref>   # card_ref = card number or UUID
```

`app-cli open` invokes the `kanban://` scheme handler, routing to the running app instance or launching it. On headless systems without a registered scheme handler, prints a warning and exits gracefully.

### URL generation in CLI output

CLI commands that create boards and cards include the `kanban://` URL in their default output. In `--json` mode, a `deep_link` field is included:

```
$ app-cli card create my-board "Deploy API" --json
{
  "id": "<uuid>",
  "number": "a7f3-1",
  "board_id": "my-board",
  "hlc": "...",
  "deep_link": "kanban://card/my-board/a7f3-1"
}
```

---

## Phased Integration

These additions map to the base plan's phases as follows:

| Addition | Earliest Phase | Notes |
|---|---|---|
| Human-readable card numbers | Phase 1 | Must be in core from the start — retrofitting number assignment requires a migration pass on existing data |
| Card copy/duplicate | Phase 1 | Follows standard card creation path; broadcast to peers like any card creation |
| Missing CLI commands (comments, checklists) | Phase 1 | Complete CLI parity before Phase 2 |
| Protocol version negotiation | Phase 2 | Required before any multi-peer testing begins |
| Undo/Redo | Phase 2 | Depends on stable CRDT ops from Phase 1; undo stack wired into all core operations |
| @Mentions | Phase 3 | Depends on stable sync + member list from Phase 2 |
| Card linking | Phase 3 | Depends on card numbers from Phase 1 and stable sync from Phase 2 |
| Peer presence indicators | Phase 3 | Depends on stable Iroh gossip from Phase 2 |
| Deep link URL scheme | Phase 4 | Requires packaged app for OS scheme registration |

---

## Open Questions (new, from these additions)

1. **Undo depth per-user vs per-board:** Should the depth limit apply per-user-per-board (simpler) or per-board total (controls total SQLite growth)? Default is per-user-per-board.
2. **Mention alias uniqueness enforcement:** Should `kanban-core` reject a member profile update that creates an alias collision, or resolve collisions in the GUI disambiguation picker only?
3. **Card number gaps:** Deleted cards leave gaps in the actor sequence (e.g., `a7f3-1`, `a7f3-3` after deleting `a7f3-2`). This is acceptable — card numbers are identifiers, not counts.
4. **Presence opt-out:** Should users be able to disable broadcasting their presence (privacy mode)? Not in scope for MVP but worth deciding before the protocol feature flag `presence_v1` is finalized.
