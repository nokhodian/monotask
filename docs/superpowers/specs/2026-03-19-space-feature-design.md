# Space Feature Design Spec

## Overview

A **Space** is a named, shareable container that holds multiple Kanban boards and a group of peers. Users can create as many Spaces as they want locally. Sharing a Space gives invited peers access to all boards that have been opt-in added to it. Real-time P2P sync is Phase 2; this spec covers the local MVP with cryptographic invite infrastructure that makes Phase 2 a drop-in.

---

## Goals

- Users can create local Spaces and opt-in boards to them
- Users can generate cryptographically-signed invite tokens (base58, file, or QR)
- Invited users can join a Space by importing a `.space` file (which embeds the SpaceDoc snapshot) or a raw base58 token (stub Space only — no snapshot without the file)
- Space owners can kick members
- User identity: Ed25519 keypair (auto-generated, or imported from SSH key), plus a display name and avatar
- Full profile (name, avatar) is embedded in the Space's shared CRDT doc, ready for P2P sync in Phase 2
- All features accessible from both the Tauri UI and the CLI (QR output is UI-only — terminals cannot render inline PNG)

## Non-Goals (Phase 2)

- Real-time P2P sync (Iroh/QUIC)
- Conflict resolution across peers
- Push notifications of board changes
- Per-board access control within a Space
- Deleting Spaces (shared CRDT documents; deletion semantics in a distributed context deferred to Phase 2)

---

## Data Model

### SQLite Tables (new, in `kanban-storage`)

```sql
CREATE TABLE spaces (
    id              TEXT PRIMARY KEY,   -- standard hyphenated UUID, e.g. "550e8400-e29b-41d4-a716-446655440000"
    name            TEXT NOT NULL CHECK(length(name) >= 1 AND length(name) <= 255),
    owner_pubkey    TEXT NOT NULL,      -- hex-encoded ed25519 public key (64 hex chars = 32 bytes)
    created_at      INTEGER NOT NULL,   -- unix timestamp seconds
    automerge_bytes BLOB NOT NULL
);

CREATE TABLE space_members (
    space_id     TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    pubkey       TEXT NOT NULL,         -- hex-encoded ed25519 public key (64 hex chars)
    display_name TEXT,
    avatar_blob  BLOB,                  -- raw image bytes; NULL if not set
    kicked       INTEGER NOT NULL DEFAULT 0,  -- cache of SpaceDoc.members[pubkey].kicked; SpaceDoc is authoritative
    PRIMARY KEY (space_id, pubkey)
);

CREATE TABLE space_boards (
    space_id     TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    board_id     TEXT NOT NULL,  -- intentionally no FK to boards(id): remote board IDs may not exist locally yet
    PRIMARY KEY (space_id, board_id)
);

CREATE TABLE space_invites (
    token_hash   TEXT PRIMARY KEY,      -- SHA-256 hex of the raw 120-byte token
    space_id     TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    created_at   INTEGER NOT NULL,      -- unix timestamp seconds
    expires_at   INTEGER,               -- unix timestamp seconds; NULL = never expires
    revoked      INTEGER NOT NULL DEFAULT 0
);

-- Single-row table (always pk = 'local') representing the local user's active identity.
CREATE TABLE user_profile (
    pk           TEXT PRIMARY KEY DEFAULT 'local',   -- always 'local'
    pubkey       TEXT NOT NULL,         -- hex-encoded ed25519 public key of the active identity
    display_name TEXT,
    avatar_blob  BLOB,
    ssh_key_path TEXT                   -- source path of SSH private key if imported; NULL if auto-generated
);
```

**Identity replacement:** `user_profile` always has at most one row (`pk = 'local'`). Replacing the identity (via `import_ssh_key`) does an `INSERT OR REPLACE INTO user_profile` with the new pubkey. This avoids orphaned rows.

**Active token invariant (DB enforced):**
```sql
CREATE UNIQUE INDEX space_invites_one_active
    ON space_invites (space_id)
    WHERE revoked = 0;
```
This prevents multiple non-revoked tokens for the same Space at the DB level. `generate_invite` must call `revoke_all_invites` before `insert_invite` to avoid a constraint violation.

**Board deletion cascade:** When a board is deleted from the `boards` table, the corresponding `space_boards` rows are deleted via `ON DELETE CASCADE`.

**`space_members.kicked` is a denormalized cache** of `SpaceDoc.members[pubkey].kicked`. The SpaceDoc is always authoritative. `kick_member` (Tauri command) must: (a) call `kick_member` on the SpaceDoc, (b) save the updated `automerge_bytes`, and (c) update `space_members.kicked = 1` in SQL. On load, `kicked` in the returned `Member` struct is read from the SpaceDoc (not from SQL cache).

SQLite is the fast query layer for the local MVP. The `automerge_bytes` column stores the CRDT payload that Phase 2 will transmit over Iroh.

### Automerge `SpaceDoc` (per Space)

Stored in `spaces.automerge_bytes`. All avatar data is stored as base64-encoded strings inside Automerge.

```
name:         String
owner_pubkey: String                            -- hex ed25519 pubkey

members: Map<pubkey → MemberProfile>
    MemberProfile {
        display_name: String        -- empty string if not set
        avatar_b64:   String        -- base64-encoded image bytes; empty string if not set
        kicked:       bool          -- authoritative kicked status
    }

boards: Map<board_id → bool>                   -- Map for idempotent concurrent adds (true = present)
```

**Why `Map` for boards:** Using `List` risks duplicate entries on concurrent inserts. A `Map<board_id → bool>` makes `add_board_ref` naturally idempotent — inserting the same board ID twice is a no-op.

**Avatar normalization across layers:**
- SQL (`space_members.avatar_blob`, `user_profile.avatar_blob`): raw bytes (`Vec<u8>`)
- SpaceDoc: base64 string (`avatar_b64`)
- Rust types: `Option<Vec<u8>>`
- Conversion: `base64::encode(bytes)` when writing to SpaceDoc; `base64::decode(s)` when reading from SpaceDoc into SQL/Rust

This is the replication unit for Phase 2. All mutations happen through Automerge so concurrent edits merge cleanly.

### Shared Rust Types (new in `kanban-core`)

```rust
pub struct Space {
    pub id: String,
    pub name: String,
    pub owner_pubkey: String,
    pub members: Vec<Member>,
    pub boards: Vec<String>,
}

pub struct Member {
    pub pubkey: String,
    pub display_name: Option<String>,
    pub avatar_blob: Option<Vec<u8>>,
    pub kicked: bool,
}

/// Profile data embedded into SpaceDoc.members when a user joins or creates a Space.
pub struct MemberProfile {
    pub display_name: String,   // empty string if not set
    pub avatar_b64: String,     // base64-encoded bytes; empty string if not set
    pub kicked: bool,
}

pub struct UserProfile {
    pub pubkey: String,
    pub display_name: Option<String>,
    pub avatar_blob: Option<Vec<u8>>,
    pub ssh_key_path: Option<String>,
}

pub struct InviteMetadata {
    pub space_id: String,       // hyphenated UUID string
    pub owner_pubkey: String,   // hex ed25519 pubkey
    pub timestamp: u64,         // unix timestamp seconds
    pub token_hash: String,     // SHA-256 hex of the raw 120-byte token — used for DB policy lookup
}
```

**Note:** `Identity` and `AutoCommit` are existing types. `Identity` is defined in `kanban-crypto`. `AutoCommit` is `automerge::AutoCommit`.

---

## Invite Token Format

**Binary layout (120 bytes total):**
```
space_id (16B)       -- RFC 4122 UUID bytes (big-endian, as returned by uuid::Uuid::as_bytes())
owner_pubkey (32B)   -- raw ed25519 public key bytes
timestamp (8B)       -- u64 unix seconds, little-endian
signature (64B)      -- ed25519 signature over the preceding 56 bytes
```
`16 + 32 + 8 + 64 = 120 bytes`

**UUID encoding:** `spaces.id` stores the standard hyphenated UUID text form. The binary token encodes the same UUID as its 16 raw bytes via `uuid::Uuid::parse_str(id)?.as_bytes()`. `verify_invite_token_signature` converts back via `uuid::Uuid::from_bytes(bytes).to_string()`.

**Base58 length:** base58-encoding 120 bytes yields approximately 162–165 characters. UI input fields must accommodate up to 165 characters.

**Expiry:** Tokens have an optional expiry stored in `space_invites.expires_at`. Default when generating: no expiry (`expires_at = NULL`).

**Active token invariant:** At most one non-revoked token per Space at any time. `generate_invite` implicitly revokes all active tokens for the Space before inserting the new row. `revoke_invite(space_id)` sets `revoked = 1` on all active tokens for the Space.

**Serialization:**
- Default: base58 string (162–165 chars) — copy/paste via any channel
- File: `.space` — JSON `{ "token": "<base58>", "space_name": "<name>", "space_doc": "<base64 automerge_bytes>" }` — the full SpaceDoc is embedded so joiners receive the owner's current snapshot
- QR: base58 string encoded as QR PNG (returned as base64 from Tauri; not available in CLI)

**Revocation:** `space_invites.revoked = 1` — enforced locally. In Phase 2, revoked tokens are broadcast via the Space CRDT doc.

---

## Identity

**Active identity resolution at startup:**
1. Read the single `user_profile` row (`pk = 'local'`) from the database
2. If the row exists and `ssh_key_path` is set and the file exists: load the Ed25519 private key from that path
3. If the row exists and `ssh_key_path` is NULL: load the auto-generated key from `{app_data_dir}/identity.key`
4. If no `user_profile` row exists: generate a new Ed25519 keypair, save private key bytes to `{app_data_dir}/identity.key`, insert `user_profile` row with `pubkey = <hex>`, `ssh_key_path = NULL`

**Refactoring required:** The existing identity-loading block in `kanban-tauri/src-tauri/src/main.rs` (which loads directly from `identity.key` without consulting `user_profile`) must be replaced with this 4-step resolution. This is an explicit implementation task.

**`import_ssh_key` behavior:** Reads the Ed25519 private key from the specified path (or `~/.ssh/id_ed25519` if `None`). SSH private key files use the OpenSSH wire format — the implementation must parse this envelope (via the `ssh-key` crate or equivalent) to extract the raw 32-byte Ed25519 scalar. The raw scalar bytes are written to `{app_data_dir}/identity.key`. Does `INSERT OR REPLACE INTO user_profile (pk, pubkey, ssh_key_path) VALUES ('local', <hex>, <path>)`. Returns the hex-encoded public key. If the file does not exist or is not a valid Ed25519 key, returns an error without modifying the active identity.

**Profile:** `user_profile` holds display name and avatar for the local user. These are embedded as a `MemberProfile` into `SpaceDoc.members` whenever the user creates or joins a Space.

---

## Components

### `kanban-core/src/space.rs` (new module)

```rust
pub fn create_space_doc(name: &str, owner_pubkey: &str) -> Result<AutoCommit>
pub fn add_member(doc: &mut AutoCommit, pubkey: &str, profile: &MemberProfile) -> Result<()>
pub fn kick_member(doc: &mut AutoCommit, pubkey: &str) -> Result<()>   // sets member.kicked = true in SpaceDoc
pub fn add_board_ref(doc: &mut AutoCommit, board_id: &str) -> Result<()>   // Map insert; idempotent
pub fn remove_board_ref(doc: &mut AutoCommit, board_id: &str) -> Result<()>   // deletes the key from the Map (not set to false)
pub fn list_members(doc: &AutoCommit) -> Result<Vec<Member>>            // reads kicked from MemberProfile.kicked
pub fn list_board_refs(doc: &AutoCommit) -> Result<Vec<String>>
// Returns board IDs whose key is present in the Map AND has a value (not tombstoned).
// Implementation: iterate Map keys; for each key, call doc.get(&boards_obj, key) and include
// only those that return Some(_). Deleted keys in Automerge become tombstones and doc.get()
// returns None for them — do not use doc.keys() alone as it includes tombstoned entries.
```

### `kanban-crypto` (extend)

```rust
/// Pure cryptographic verification — checks ed25519 signature and base58 decoding only. No DB access.
/// Steps: (1) base58-decode token string to raw bytes, (2) verify length == 120,
///        (3) compute SHA-256 of the raw 120 bytes → token_hash (hex), (4) verify ed25519 signature.
/// Returns InviteMetadata with token_hash pre-computed from the raw bytes (not from the base58 string).
/// Error variants: InvalidBase58, InvalidLength, InvalidSignature.
pub fn verify_invite_token_signature(token: &str) -> Result<InviteMetadata>

/// Generates a base58-encoded invite token signed with the given identity.
pub fn generate_invite_token(space_id: &str, identity: &Identity) -> Result<String>

/// Reads an Ed25519 private key from the given path (or ~/.ssh/id_ed25519 if None).
/// Error variants: FileNotFound, InvalidKeyFormat.
pub fn import_ssh_identity(path: Option<&Path>) -> Result<Identity>
```

### `kanban-storage` (extend)

```rust
/// Policy enforcement — checks revocation and expiry in space_invites using InviteMetadata.token_hash.
/// `local_pubkey` is the active identity's hex pubkey; used to distinguish owner vs. joiner.
/// - If local_pubkey == metadata.owner_pubkey (owner path): checks space_invites; Err if revoked or expired.
/// - If local_pubkey != metadata.owner_pubkey (joiner path): returns Ok(()) immediately (no-op for MVP;
///   revocation enforcement deferred to Phase 2 via CRDT sync).
pub fn check_invite_policy(conn: &Connection, metadata: &InviteMetadata, local_pubkey: &str) -> Result<()>
```

**`SpaceStore` public interface:**
```rust
/// Returns lightweight summaries (no members/boards populated) — used for sidebar list.
pub fn list_spaces(conn: &Connection) -> Result<Vec<SpaceSummary>>

/// Returns fully populated Space (members + boards). Returns Err("Space not found") if space_id unknown.
pub fn get_space(conn: &Connection, space_id: &str) -> Result<Space>

pub fn create_space(conn: &Connection, id: &str, name: &str, owner_pubkey: &str, automerge_bytes: &[u8]) -> Result<()>
pub fn update_space_doc(conn: &Connection, space_id: &str, automerge_bytes: &[u8]) -> Result<()>
/// Loads the raw automerge_bytes for a Space so callers can reconstruct the AutoCommit.
pub fn load_space_doc(conn: &Connection, space_id: &str) -> Result<Vec<u8>>
pub fn upsert_member(conn: &Connection, space_id: &str, member: &Member) -> Result<()>  // INSERT OR REPLACE
pub fn set_member_kicked(conn: &Connection, space_id: &str, pubkey: &str, kicked: bool) -> Result<()>
pub fn add_board(conn: &Connection, space_id: &str, board_id: &str) -> Result<()>  // no FK to boards table; ignores unknown board IDs
pub fn remove_board(conn: &Connection, space_id: &str, board_id: &str) -> Result<()>
pub fn insert_invite(conn: &Connection, token_hash: &str, space_id: &str, expires_at: Option<i64>) -> Result<()>
pub fn revoke_all_invites(conn: &Connection, space_id: &str) -> Result<()>
```

**`SpaceSummary` (new lightweight type for list view):**
```rust
pub struct SpaceSummary {
    pub id: String,
    pub name: String,
    pub member_count: usize,
}
```
`list_spaces` uses a single SQL query joining `space_members` for the count — no N+1. `get_space` returns the fully populated `Space`. The Tauri `list_spaces` command returns `Vec<SpaceSummary>`; the sidebar displays name + count from this.

**`ProfileStore` public interface:**
```rust
pub fn get_profile(conn: &Connection) -> Result<Option<UserProfile>>
pub fn upsert_profile(conn: &Connection, profile: &UserProfile) -> Result<()>  // INSERT OR REPLACE with pk='local'
```

### `kanban-tauri/src-tauri/src/main.rs` (extend)

14 new Tauri commands (see Commands section). The existing identity-loading block must be refactored to the 4-step resolution described in the Identity section.

### `kanban-cli/src/main.rs` (extend)

`space` and `profile` subcommand groups (see Commands section).

### `kanban-tauri/src/index.html` (extend)

Space UI panels (see UI section).

---

## Commands

### Tauri Commands

| Command | Signature | Returns |
|---|---|---|
| `create_space` | `(name: String)` | `Space` |
| `list_spaces` | `()` | `Vec<SpaceSummary>` |
| `get_space` | `(space_id: String)` | `Space` |
| `generate_invite` | `(space_id: String)` | `String` (base58) |
| `revoke_invite` | `(space_id: String)` | `()` |
| `export_invite_file` | `(space_id: String, path: String)` | `()` |
| `get_invite_qr` | `(space_id: String)` | `String` (base64 PNG) |
| `import_invite` | `(token_or_path: String)` | `Space` |
| `add_board_to_space` | `(space_id: String, board_id: String)` | `()` |
| `remove_board_from_space` | `(space_id: String, board_id: String)` | `()` |
| `kick_member` | `(space_id: String, pubkey: String)` | `()` |
| `get_my_profile` | `()` | `UserProfile` |
| `update_my_profile` | `(display_name: String, avatar: Option<Vec<u8>>)` | `()` |
| `import_ssh_key` | `(path: Option<String>)` | `String` (hex pubkey) |

**`create_space` behavior:** Creates the SpaceDoc via `create_space_doc(name, local_pubkey)`, adds the local user as the first member via `add_member(doc, local_pubkey, local_profile)`, persists to SQL, and returns the resulting `Space` (which contains one member: the creator).

**`import_invite` input disambiguation:** If `token_or_path` ends with `.space` or resolves to an existing file path on disk, it is treated as a `.space` file (JSON parsed for `token` and `space_name` fields). Otherwise it is treated as a raw base58 token string.

**`import_invite` full flow:**
1. Parse token string and optionally `space_name` + `space_doc` from `.space` file, or accept raw base58 token
2. Call `verify_invite_token_signature(token)` → `InviteMetadata` (errors: `InvalidBase58`, `InvalidLength`, `InvalidSignature`)
3. Call `check_invite_policy(conn, &metadata, local_pubkey)` — no-op if local user is not the Space owner; owner-side check for revoked/expired tokens
4. Check `space_members` for a row with `(space_id = metadata.space_id, pubkey = local_user_pubkey)` — if found, call `SpaceStore::get_space(conn, space_id)` and return the result (no-op, idempotent)
5. **If `.space` file with `space_doc` field:** decode the base64 `space_doc` bytes and load as the SpaceDoc (`AutoCommit::load(bytes)`) — this is the owner's snapshot; all existing members and boards are preserved
6. **If raw base58 token (no `space_doc`):** call `create_space_doc(name, owner_pubkey)` where `name` = `.space` file's `space_name` if present and non-empty, otherwise `"Shared Space"`; call `add_member(doc, owner_pubkey, empty_profile)` — owner stub with empty profile
7. Call `add_member(doc, local_pubkey, local_profile)` — local user added with their full profile (idempotent Map insert)
8. Insert row into `spaces` with the SpaceDoc bytes; for each member in SpaceDoc, decode `avatar_b64` from base64 to `Vec<u8>` and call `SpaceStore::upsert_member`; insert `space_boards` rows from `list_board_refs(doc)` (file path only — no boards for raw token path; `space_boards` has no FK to `boards`, so remote board IDs are stored even if the local `boards` table doesn't have them yet)
9. Return the resulting `Space` struct

**`revoke_invite` behavior:** Calls `SpaceStore::revoke_all_invites(conn, space_id)`.

**`generate_invite` behavior:** Calls `revoke_all_invites` first (implicit revocation), then generates and persists the new token, returning the base58 string.

**Revoke & Regenerate UI flow:** "Revoke & Regenerate" button calls `generate_invite(space_id)` only (which implicitly revokes). The invite tab refreshes with the new token and QR.

**`update_my_profile` behavior:** Stores the updated display name and avatar in `user_profile` (via `ProfileStore::upsert_profile`). Then for each Space in `list_spaces`: (1) load the AutoCommit via `SpaceStore::load_space_doc(conn, space_id)` + `AutoCommit::load(bytes)`, (2) call `add_member(doc, local_pubkey, updated_profile)` (idempotent Map upsert in SpaceDoc), (3) call `SpaceStore::update_space_doc(conn, space_id, &doc.save())`, (4) call `SpaceStore::upsert_member(conn, space_id, &updated_member)` to keep SQL cache consistent. This ensures both SpaceDoc and SQL rows reflect the latest profile.

**Member data source in `get_space`:** `SpaceStore::get_space` reads member rows from `space_members` SQL (not from the SpaceDoc). The SQL rows are the authoritative source for Tauri command responses. The SpaceDoc is the authoritative source for CRDT state. Operations that mutate SpaceDoc (kick, add_member, update_my_profile) must always update both SQL and SpaceDoc in the same logical operation.

**Sidebar ownership:** `SpaceSummary` does not include `owner_pubkey` and the sidebar does not display any ownership indicator. The "Kick" button visibility (owner-only) is determined by comparing `Space.owner_pubkey` to `local_pubkey` on the Space detail panel (which calls `get_space`).

### CLI Subcommands

```
space create <name>
space list
space info <space-id>
space invite generate <space-id>
space invite export <space-id> <output-file>
space invite revoke <space-id>
space join <token-or-file>
space boards add <space-id> <board-id>
space boards remove <space-id> <board-id>
space boards list <space-id>
space members list <space-id>
space members kick <space-id> <pubkey>

profile show
profile set-name <name>
profile set-avatar <path>            # loads image file at path as avatar blob
profile import-ssh-key [path]        # defaults to ~/.ssh/id_ed25519
```

**`space join` input disambiguation:** same heuristic as `import_invite` — file path if the argument ends with `.space` or resolves to an existing file; raw base58 token otherwise.

**QR code:** Not available as a CLI command. QR output is UI-only (terminals cannot render inline PNG).

---

## UI

### Spaces Sidebar Panel

Left of the board dashboard. Shows list of local Spaces with name and member count. "New Space" button opens a creation modal.

### Space Creation Modal

Single name input field (1–255 characters). On submit: creates Space, auto-generates first invite token, navigates to Space detail.

### Space Detail / Settings Panel

Replaces the board dashboard when a Space is selected. Three tabs:

**Members tab**
- List rows: avatar initial + display name + truncated pubkey
- "Kick" button visible to Space owner only
- Kicked members shown greyed out

**Boards tab**
- List of boards currently in this Space with "Remove" button
- "Add Board" dropdown showing local boards not yet in this Space

**Invite tab**
- If no active token exists: show "Generate Invite" button; token field and QR are hidden/placeholder
- If active token exists: read-only text field with base58 token + "Copy" button (field accommodates up to 165 chars); "Export .space file" button; QR code image (inline PNG from `get_invite_qr`); "Revoke & Regenerate" button (calls `generate_invite`)

### Board Dashboard Changes

- Each board card shows a Space badge (e.g. `[Work]`) for every Space it belongs to
- Board settings (`⚙`) gains "Add to Space" and "Remove from Space" options

### Profile Settings

New section in a global settings modal (accessible from sidebar footer or a `⚙` icon):
- Display name text input
- Avatar upload (shows initials if no avatar set)
- Public key display: truncated hex + "Copy full key" button
- "Import SSH key" button with optional path input field

---

## Error Handling

| Scenario | Behavior |
|---|---|
| Invalid invite token (bad base58 / bad signature / wrong length) | Return error: "Invalid or tampered invite token" |
| Revoked invite (owner-side check only; joiner-side is no-op for MVP) | Return error: "This invite has been revoked" |
| Expired invite (`now > expires_at`, owner-side only) | Return error: "This invite has expired" |
| SSH key not found at path | Return error with path; do not modify active identity |
| SSH key file is not a valid Ed25519 key | Return error: "Not a valid Ed25519 key"; do not modify active identity |
| Kicking non-member | No-op (idempotent) |
| Adding board already in Space | No-op (idempotent; `SpaceDoc.boards` is a Map) |
| Removing a board not in Space | No-op (idempotent) |
| Joining a Space you already belong to | Return current Space state (no-op) |
| `import_invite` base58 token with no name | Create Space with placeholder name `"Shared Space"` |
| Adding a non-existent board to a Space | Stored in `space_boards` without error (no FK; remote board IDs are valid) |
| `create_space` with empty or too-long name | Return error: "Name must be 1–255 characters" |
| `export_invite_file` to a path that already exists | Overwrite silently |
| `get_space` with unknown `space_id` | Return error: "Space not found" |

---

## Testing

- Unit tests in `kanban-core/src/space.rs`: create/add-member/kick/add-board/remove-board; verify `kicked` reads from `MemberProfile.kicked`; verify `boards` Map deduplication
- Unit tests in `kanban-crypto`: `generate_invite_token` + `verify_invite_token_signature` round-trip; tampered-token rejection (bit-flip in signature); `InvalidBase58` and `InvalidLength` error paths; SSH key import (valid key, missing file, non-Ed25519 file)
- Integration tests in `kanban-storage`: SQL migrations run cleanly; `SpaceStore` CRUD; `ProfileStore` INSERT OR REPLACE; `check_invite_policy` returns Ok for active owner-side token, Err for revoked, Err for expired, Ok for joiner (no row in `space_invites`)
- CLI integration tests: `space create`, `space invite generate`, `space join` (raw base58 and `.space` file), `space boards add/remove/list`, `space members kick`, `profile set-name`, `profile set-avatar`, `profile import-ssh-key`
- Tauri command layer tested via manual UI smoke test (Tauri has no automated test harness; CLI tests cover the same core logic paths via shared `kanban-core`/`kanban-storage` code)
- Manual UI smoke test (happy paths): create Space → add board → generate invite → copy token → view QR → export `.space` file → revoke & regenerate → kick member → profile settings → import SSH key → verify profile shows new pubkey
- Manual UI smoke test (error paths): import malformed base58 token → import revoked token → import token with `expires_at` in the past → attempt to add non-existent board

---

## Phase 2 Hook Points

The design deliberately leaves these extension points:

1. `automerge_bytes` in `spaces` table → feed directly into Iroh document sync as the Space namespace payload
2. `InviteMetadata.owner_pubkey` → used for Iroh peer authentication. Iroh node IDs are Ed25519 public keys; `kanban-crypto` already uses Ed25519, so the key format is compatible with Iroh without conversion.
3. `space_invites.revoked` → propagated via CRDT merge in Phase 2; when a synced SpaceDoc shows `MemberProfile.kicked = true` for the local user, the app should treat that Space as read-only and surface a "You have been removed from this Space" notice
4. `SpaceDoc.members` map → member profiles and kicked status sync automatically when the Space doc merges
5. **Board document sync:** `SpaceDoc.boards` holds board IDs. In Phase 2, each board ID maps to an Iroh namespace derived deterministically as `SHA-256(space_id || board_id)`. Iroh syncs both the Space doc and each board doc independently. Each board's existing `automerge_bytes` column (in the `boards` table) is the payload for that namespace.
