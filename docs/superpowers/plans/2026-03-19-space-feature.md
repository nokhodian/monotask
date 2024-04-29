# Space Feature Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add local Spaces — named containers holding multiple boards — with cryptographic invite tokens (Ed25519-signed, base58/file), member profiles, and kick support, ready for Phase 2 P2P sync.

**Architecture:** Layered: `kanban-core/src/space.rs` holds Automerge SpaceDoc CRDT ops and shared types; `kanban-storage/src/space.rs` holds SQL SpaceStore/ProfileStore; `kanban-crypto` gains invite token generation/verification and SSH key import. Tauri and CLI surfaces wire everything together. All mutations update both SpaceDoc (automerge_bytes) and SQL cache atomically.

**Tech Stack:** Rust 2021, Automerge 0.5, rusqlite 0.31, ed25519-dalek 2, bs58, sha2, ssh-key, base64. QR rendering via qrcode.js CDN in the UI (no Rust image deps).

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `Cargo.toml` (workspace) | Modify | Add `bs58`, `sha2`, `ssh-key`, `base64` deps |
| `crates/kanban-core/Cargo.toml` | Modify | Add `base64` |
| `crates/kanban-crypto/Cargo.toml` | Modify | Add `kanban-core`, `uuid`, `bs58`, `sha2`, `ssh-key` |
| `crates/kanban-storage/Cargo.toml` | No change | `kanban-core` dep already present |
| `crates/kanban-cli/Cargo.toml` | Modify | Add `automerge`, `uuid`, `base64` (not yet in file) |
| `crates/kanban-core/src/space.rs` | Create | Shared types + SpaceDoc CRDT functions |
| `crates/kanban-core/src/lib.rs` | Modify | `pub mod space;` |
| `crates/kanban-crypto/src/lib.rs` | Modify | `generate_invite_token`, `verify_invite_token_signature`, `import_ssh_identity`, new error variants |
| `crates/kanban-storage/src/schema.rs` | Modify | 5 new tables + partial unique index |
| `crates/kanban-storage/src/space.rs` | Create | `SpaceStore` + `ProfileStore` + `check_invite_policy` |
| `crates/kanban-storage/src/lib.rs` | Modify | `pub mod space;` |
| `crates/kanban-tauri/src-tauri/src/main.rs` | Modify | Refactor identity loading + 14 new commands |
| `crates/kanban-cli/src/main.rs` | Modify | `space` + `profile` subcommand groups |
| `crates/kanban-tauri/src/index.html` | Modify | Spaces sidebar, detail panel, invite tab, profile settings |

---

## Task 1: Add workspace dependencies

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/kanban-core/Cargo.toml`
- Modify: `crates/kanban-crypto/Cargo.toml`

- [ ] **Step 1: Add workspace deps**

In `Cargo.toml` under `[workspace.dependencies]`, add:
```toml
bs58    = "0.5"
sha2    = "0.10"
ssh-key = { version = "0.6", features = ["ed25519"] }
base64  = "0.22"
hex     = "0.4"
```

- [ ] **Step 2: Add kanban-core deps**

In `crates/kanban-core/Cargo.toml` under `[dependencies]`:
```toml
base64 = { workspace = true }
```

- [ ] **Step 3: Add kanban-crypto deps**

In `crates/kanban-crypto/Cargo.toml` under `[dependencies]`:
```toml
kanban-core = { path = "../kanban-core" }
uuid        = { workspace = true }
bs58        = { workspace = true }
sha2        = { workspace = true }
ssh-key     = { workspace = true }
hex         = { workspace = true }
```

- [ ] **Step 4: Verify it compiles**

```bash
cd /Users/morteza/Desktop/monoes/monotask
cargo check -p kanban-core -p kanban-crypto
```
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/kanban-core/Cargo.toml crates/kanban-crypto/Cargo.toml
git commit -m "chore: add bs58, sha2, ssh-key, base64 workspace deps"
```

---

## Task 2: Shared types + SpaceDoc CRDT functions in kanban-core

**Files:**
- Create: `crates/kanban-core/src/space.rs`
- Modify: `crates/kanban-core/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/kanban-core/src/space.rs` with tests first:

```rust
use automerge::{AutoCommit, ObjType, ReadDoc, transaction::Transactable};
use serde::{Deserialize, Serialize};

// ── Shared types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceSummary {
    pub id: String,
    pub name: String,
    pub member_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub owner_pubkey: String,
    pub members: Vec<Member>,
    pub boards: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub pubkey: String,
    pub display_name: Option<String>,
    pub avatar_blob: Option<Vec<u8>>,
    pub kicked: bool,
}

/// Profile embedded into SpaceDoc.members map entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberProfile {
    pub display_name: String,  // empty string if not set
    pub avatar_b64: String,    // base64-encoded bytes; empty string if not set
    pub kicked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub pubkey: String,
    pub display_name: Option<String>,
    pub avatar_blob: Option<Vec<u8>>,
    pub ssh_key_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteMetadata {
    pub space_id: String,       // hyphenated UUID
    pub owner_pubkey: String,   // hex ed25519 pubkey
    pub timestamp: u64,         // unix seconds
    pub token_hash: String,     // SHA-256 hex of raw 120-byte token
}

// ── CRDT helpers ──────────────────────────────────────────────────────────────

fn get_members_map(doc: &AutoCommit) -> crate::Result<automerge::ObjId> {
    match doc.get(automerge::ROOT, "members")? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::InvalidDocument("space missing members map".into())),
    }
}

fn get_boards_map(doc: &AutoCommit) -> crate::Result<automerge::ObjId> {
    match doc.get(automerge::ROOT, "boards")? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::InvalidDocument("space missing boards map".into())),
    }
}

// ── Public CRDT API ───────────────────────────────────────────────────────────

pub fn create_space_doc(name: &str, owner_pubkey: &str) -> crate::Result<AutoCommit> {
    let mut doc = AutoCommit::new();
    doc.put(automerge::ROOT, "name", name)?;
    doc.put(automerge::ROOT, "owner_pubkey", owner_pubkey)?;
    doc.put_object(automerge::ROOT, "members", ObjType::Map)?;
    doc.put_object(automerge::ROOT, "boards", ObjType::Map)?;
    Ok(doc)
}

pub fn add_member(doc: &mut AutoCommit, pubkey: &str, profile: &MemberProfile) -> crate::Result<()> {
    let members = get_members_map(doc)?;
    let entry = match doc.get(&members, pubkey)? {
        Some((_, id)) => id,
        None => doc.put_object(&members, pubkey, ObjType::Map)?,
    };
    doc.put(&entry, "display_name", profile.display_name.as_str())?;
    doc.put(&entry, "avatar_b64", profile.avatar_b64.as_str())?;
    doc.put(&entry, "kicked", profile.kicked)?;
    Ok(())
}

pub fn kick_member(doc: &mut AutoCommit, pubkey: &str) -> crate::Result<()> {
    let members = get_members_map(doc)?;
    if let Some((_, entry)) = doc.get(&members, pubkey)? {
        doc.put(&entry, "kicked", true)?;
    }
    Ok(())
}

pub fn add_board_ref(doc: &mut AutoCommit, board_id: &str) -> crate::Result<()> {
    let boards = get_boards_map(doc)?;
    doc.put(&boards, board_id, true)?;
    Ok(())
}

pub fn remove_board_ref(doc: &mut AutoCommit, board_id: &str) -> crate::Result<()> {
    let boards = get_boards_map(doc)?;
    // delete tombstones the key; list_board_refs filters tombstoned keys
    if doc.get(&boards, board_id)?.is_some() {
        doc.delete(&boards, board_id)?;
    }
    Ok(())
}

pub fn list_members(doc: &AutoCommit) -> crate::Result<Vec<Member>> {
    let members = get_members_map(doc)?;
    let mut result = Vec::new();
    for key in doc.keys(&members) {
        let pubkey = key.to_string();
        if let Some((_, entry)) = doc.get(&members, &pubkey)? {
            let display_name = crate::get_string(doc, &entry, "display_name")?
                .filter(|s| !s.is_empty());
            let avatar_b64 = crate::get_string(doc, &entry, "avatar_b64")?
                .unwrap_or_default();
            let avatar_blob = if avatar_b64.is_empty() {
                None
            } else {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(&avatar_b64).ok()
            };
            let kicked = matches!(
                doc.get(&entry, "kicked")?,
                Some((automerge::Value::Scalar(s), _))
                    if matches!(s.as_ref(), automerge::ScalarValue::Boolean(true))
            );
            result.push(Member { pubkey, display_name, avatar_blob, kicked });
        }
    }
    Ok(result)
}

pub fn list_board_refs(doc: &AutoCommit) -> crate::Result<Vec<String>> {
    let boards = get_boards_map(doc)?;
    let mut result = Vec::new();
    for key in doc.keys(&boards) {
        let board_id = key.to_string();
        // Only include keys with a live value (not tombstoned)
        if doc.get(&boards, &board_id)?.is_some() {
            result.push(board_id);
        }
    }
    Ok(result)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_space_doc_has_required_fields() {
        let doc = create_space_doc("My Space", "aabbcc").unwrap();
        let name = crate::get_string(&doc, &automerge::ROOT, "name").unwrap();
        assert_eq!(name, Some("My Space".into()));
        let owner = crate::get_string(&doc, &automerge::ROOT, "owner_pubkey").unwrap();
        assert_eq!(owner, Some("aabbcc".into()));
    }

    #[test]
    fn add_and_list_members() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        let profile = MemberProfile {
            display_name: "Alice".into(),
            avatar_b64: "".into(),
            kicked: false,
        };
        add_member(&mut doc, "pk_alice", &profile).unwrap();
        let members = list_members(&doc).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].pubkey, "pk_alice");
        assert_eq!(members[0].display_name, Some("Alice".into()));
        assert!(!members[0].kicked);
    }

    #[test]
    fn kick_member_sets_kicked_true() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        let profile = MemberProfile { display_name: "Bob".into(), avatar_b64: "".into(), kicked: false };
        add_member(&mut doc, "pk_bob", &profile).unwrap();
        kick_member(&mut doc, "pk_bob").unwrap();
        let members = list_members(&doc).unwrap();
        assert!(members[0].kicked);
    }

    #[test]
    fn add_and_remove_board_ref() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        add_board_ref(&mut doc, "board-1").unwrap();
        add_board_ref(&mut doc, "board-2").unwrap();
        let boards = list_board_refs(&doc).unwrap();
        assert_eq!(boards.len(), 2);
        remove_board_ref(&mut doc, "board-1").unwrap();
        let boards = list_board_refs(&doc).unwrap();
        assert_eq!(boards.len(), 1);
        assert_eq!(boards[0], "board-2");
    }

    #[test]
    fn add_board_ref_is_idempotent() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        add_board_ref(&mut doc, "board-1").unwrap();
        add_board_ref(&mut doc, "board-1").unwrap();
        let boards = list_board_refs(&doc).unwrap();
        assert_eq!(boards.len(), 1);
    }

    #[test]
    fn add_member_is_idempotent_upsert() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        let p1 = MemberProfile { display_name: "Alice".into(), avatar_b64: "".into(), kicked: false };
        let p2 = MemberProfile { display_name: "Alice Updated".into(), avatar_b64: "".into(), kicked: false };
        add_member(&mut doc, "pk_alice", &p1).unwrap();
        add_member(&mut doc, "pk_alice", &p2).unwrap();
        let members = list_members(&doc).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].display_name, Some("Alice Updated".into()));
    }
}
```

- [ ] **Step 2: Register module in lib.rs**

In `crates/kanban-core/src/lib.rs`, add after existing `pub mod` lines:
```rust
pub mod space;
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cd /Users/morteza/Desktop/monoes/monotask
cargo test -p kanban-core -- space 2>&1 | head -30
```
Expected: compile error (file doesn't exist yet) — this confirms the test file needs to be created.

- [ ] **Step 4: Run tests (they should now pass since code and tests are in same file)**

```bash
cargo test -p kanban-core -- space::tests
```
Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-core/src/space.rs crates/kanban-core/src/lib.rs
git commit -m "feat(kanban-core): add Space types and SpaceDoc CRDT functions"
```

---

## Task 3: Invite token crypto functions in kanban-crypto

**Files:**
- Modify: `crates/kanban-crypto/src/lib.rs`

- [ ] **Step 1: Add new error variants and write failing tests**

In `crates/kanban-crypto/src/lib.rs`, **replace** the existing `CryptoError` enum entirely with the expanded one below (it currently has `InvalidKey` and `VerifyFailed` — we add invite/SSH variants):

```rust
use std::path::Path;
use sha2::{Sha256, Digest};
use kanban_core::space::InviteMetadata;

// REPLACE the existing CryptoError enum with:
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("invalid key")]
    InvalidKey,
    #[error("signature verification failed")]
    VerifyFailed,
    #[error("invalid base58 encoding")]
    InvalidBase58,
    #[error("invalid token length")]
    InvalidLength,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("file not found: {0}")]
    FileNotFound(String),
    #[error("invalid key format: {0}")]
    InvalidKeyFormat(String),
}
```

Then add the new functions and tests at the bottom of the file:

```rust
pub fn generate_invite_token(space_id: &str, identity: &Identity) -> Result<String, CryptoError> {
    let uuid = uuid::Uuid::parse_str(space_id).map_err(|_| CryptoError::InvalidKey)?;
    let space_id_bytes = *uuid.as_bytes(); // [u8; 16]
    let pubkey_bytes = identity.public_key_bytes();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut payload = [0u8; 56];
    payload[0..16].copy_from_slice(&space_id_bytes);
    payload[16..48].copy_from_slice(&pubkey_bytes);
    payload[48..56].copy_from_slice(&timestamp.to_le_bytes());

    let sig = identity.sign(&payload);

    let mut token_bytes = [0u8; 120];
    token_bytes[0..56].copy_from_slice(&payload);
    token_bytes[56..120].copy_from_slice(&sig);

    Ok(bs58::encode(token_bytes).into_string())
}

pub fn verify_invite_token_signature(token: &str) -> Result<InviteMetadata, CryptoError> {
    let bytes = bs58::decode(token)
        .into_vec()
        .map_err(|_| CryptoError::InvalidBase58)?;

    if bytes.len() != 120 {
        return Err(CryptoError::InvalidLength);
    }

    // Compute token_hash from raw bytes (not from base58 string)
    let token_hash = hex::encode(Sha256::digest(&bytes));

    let space_id_bytes: [u8; 16] = bytes[0..16].try_into().unwrap();
    let pubkey_bytes: [u8; 32] = bytes[16..48].try_into().unwrap();
    let timestamp = u64::from_le_bytes(bytes[48..56].try_into().unwrap());
    let sig_bytes = &bytes[56..120];

    // Verify signature over first 56 bytes
    Identity::verify(&pubkey_bytes, &bytes[0..56], sig_bytes)
        .map_err(|_| CryptoError::InvalidSignature)?;

    let space_id = uuid::Uuid::from_bytes(space_id_bytes).to_string();
    let owner_pubkey = hex::encode(pubkey_bytes);

    Ok(InviteMetadata { space_id, owner_pubkey, timestamp, token_hash })
}

pub fn import_ssh_identity(path: Option<&Path>) -> Result<Identity, CryptoError> {
    use ssh_key::PrivateKey;

    let key_path = match path {
        Some(p) => p.to_path_buf(),
        None => {
            let home = std::env::var("HOME").unwrap_or_default();
            std::path::PathBuf::from(home).join(".ssh").join("id_ed25519")
        }
    };

    if !key_path.exists() {
        return Err(CryptoError::FileNotFound(key_path.display().to_string()));
    }

    let pem = std::fs::read_to_string(&key_path)
        .map_err(|e| CryptoError::InvalidKeyFormat(e.to_string()))?;

    let private_key = PrivateKey::from_openssh(&pem)
        .map_err(|e| CryptoError::InvalidKeyFormat(e.to_string()))?;

    let ed25519_keypair = private_key
        .key_data()
        .ed25519()
        .ok_or_else(|| CryptoError::InvalidKeyFormat("not an Ed25519 key".into()))?;

    let secret_bytes: [u8; 32] = ed25519_keypair.private.to_bytes();
    Ok(Identity::from_secret_bytes(&secret_bytes))
}

#[cfg(test)]
mod invite_tests {
    use super::*;

    #[test]
    fn generate_and_verify_token_roundtrip() {
        let identity = Identity::generate();
        let space_id = uuid::Uuid::new_v4().to_string();
        let token = generate_invite_token(&space_id, &identity).unwrap();
        let meta = verify_invite_token_signature(&token).unwrap();
        assert_eq!(meta.space_id, space_id);
        assert_eq!(meta.owner_pubkey, identity.public_key_hex());
        assert!(!meta.token_hash.is_empty());
    }

    #[test]
    fn verify_rejects_tampered_token() {
        let identity = Identity::generate();
        let space_id = uuid::Uuid::new_v4().to_string();
        let token = generate_invite_token(&space_id, &identity).unwrap();
        let mut bytes = bs58::decode(&token).into_vec().unwrap();
        bytes[60] ^= 0xFF; // flip bits in signature
        let tampered = bs58::encode(&bytes).into_string();
        assert!(matches!(
            verify_invite_token_signature(&tampered),
            Err(CryptoError::InvalidSignature)
        ));
    }

    #[test]
    fn verify_rejects_invalid_base58() {
        assert!(matches!(
            verify_invite_token_signature("not-valid-base58!!!"),
            Err(CryptoError::InvalidBase58)
        ));
    }

    #[test]
    fn verify_rejects_wrong_length() {
        let short = bs58::encode(vec![0u8; 50]).into_string();
        assert!(matches!(
            verify_invite_token_signature(&short),
            Err(CryptoError::InvalidLength)
        ));
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cd /Users/morteza/Desktop/monoes/monotask
cargo test -p kanban-crypto -- invite_tests
```
Expected: all 4 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/kanban-crypto/src/lib.rs crates/kanban-crypto/Cargo.toml
git commit -m "feat(kanban-crypto): add invite token generation/verification and SSH key import"
```

---

## Task 4: Space SQL schema

**Files:**
- Modify: `crates/kanban-storage/src/schema.rs`

- [ ] **Step 1: Write failing migration test**

Add to `crates/kanban-storage/src/schema.rs` at the bottom:
```rust
#[cfg(test)]
mod space_schema_tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn space_tables_created_by_migration() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        // Verify all 5 tables exist
        for table in &["spaces", "space_members", "space_boards", "space_invites", "user_profile"] {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            ).unwrap();
            assert_eq!(count, 1, "table {} not found", table);
        }
        // Verify unique index on space_invites
        let idx_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='space_invites_one_active'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(idx_count, 1);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p kanban-storage -- space_schema_tests
```
Expected: FAIL — tables don't exist yet.

- [ ] **Step 3: Add the new tables to run_migrations**

In `crates/kanban-storage/src/schema.rs`, extend the `execute_batch` call. Add these lines before the final `COMMIT;`:

```sql
        CREATE TABLE IF NOT EXISTS spaces (
            id              TEXT PRIMARY KEY,
            name            TEXT NOT NULL CHECK(length(name) >= 1 AND length(name) <= 255),
            owner_pubkey    TEXT NOT NULL,
            created_at      INTEGER NOT NULL,
            automerge_bytes BLOB NOT NULL
        );

        CREATE TABLE IF NOT EXISTS space_members (
            space_id     TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
            pubkey       TEXT NOT NULL,
            display_name TEXT,
            avatar_blob  BLOB,
            kicked       INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (space_id, pubkey)
        );

        CREATE TABLE IF NOT EXISTS space_boards (
            space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
            board_id TEXT NOT NULL,
            PRIMARY KEY (space_id, board_id)
        );

        CREATE TABLE IF NOT EXISTS space_invites (
            token_hash TEXT PRIMARY KEY,
            token      TEXT NOT NULL,
            space_id   TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
            created_at INTEGER NOT NULL,
            expires_at INTEGER,
            revoked    INTEGER NOT NULL DEFAULT 0
        );

        CREATE UNIQUE INDEX IF NOT EXISTS space_invites_one_active
            ON space_invites (space_id) WHERE revoked = 0;

        CREATE TABLE IF NOT EXISTS user_profile (
            pk           TEXT PRIMARY KEY DEFAULT 'local',
            pubkey       TEXT NOT NULL,
            display_name TEXT,
            avatar_blob  BLOB,
            ssh_key_path TEXT
        );
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p kanban-storage -- space_schema_tests
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-storage/src/schema.rs
git commit -m "feat(kanban-storage): add Space SQL schema (5 tables + partial unique index)"
```

---

## Task 5: SpaceStore + ProfileStore in kanban-storage

**Files:**
- Create: `crates/kanban-storage/src/space.rs`
- Modify: `crates/kanban-storage/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/kanban-storage/src/space.rs`:

```rust
use rusqlite::{Connection, params};
use kanban_core::space::{
    InviteMetadata, Member, Space, SpaceSummary, UserProfile,
};
use crate::StorageError;

// ── SpaceStore ────────────────────────────────────────────────────────────────

pub fn list_spaces(conn: &Connection) -> Result<Vec<SpaceSummary>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, COUNT(m.pubkey) as cnt
         FROM spaces s
         LEFT JOIN space_members m ON m.space_id = s.id AND m.kicked = 0
         GROUP BY s.id, s.name
         ORDER BY s.created_at ASC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(SpaceSummary {
            id: row.get(0)?,
            name: row.get(1)?,
            member_count: row.get::<_, i64>(2)? as usize,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::Sqlite)
}

pub fn get_space(conn: &Connection, space_id: &str) -> Result<Space, StorageError> {
    let (name, owner_pubkey) = conn.query_row(
        "SELECT name, owner_pubkey FROM spaces WHERE id = ?1",
        [space_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(format!("Space {space_id}")),
        other => StorageError::Sqlite(other),
    })?;

    let mut stmt = conn.prepare(
        "SELECT pubkey, display_name, avatar_blob, kicked FROM space_members WHERE space_id = ?1"
    )?;
    let members: Vec<Member> = stmt.query_map([space_id], |row| {
        let display_name: Option<String> = row.get(1)?;
        let avatar_blob: Option<Vec<u8>> = row.get(2)?;
        let kicked: bool = row.get::<_, i32>(3)? != 0;
        Ok(Member {
            pubkey: row.get(0)?,
            display_name: display_name.filter(|s| !s.is_empty()),
            avatar_blob,
            kicked,
        })
    })?.collect::<Result<Vec<_>, _>>()?;

    let mut stmt2 = conn.prepare(
        "SELECT board_id FROM space_boards WHERE space_id = ?1"
    )?;
    let boards: Vec<String> = stmt2.query_map([space_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Space { id: space_id.to_string(), name, owner_pubkey, members, boards })
}

pub fn create_space(
    conn: &Connection,
    id: &str,
    name: &str,
    owner_pubkey: &str,
    automerge_bytes: &[u8],
) -> Result<(), StorageError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "INSERT INTO spaces (id, name, owner_pubkey, created_at, automerge_bytes)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, name, owner_pubkey, now, automerge_bytes],
    )?;
    Ok(())
}

pub fn load_space_doc(conn: &Connection, space_id: &str) -> Result<Vec<u8>, StorageError> {
    conn.query_row(
        "SELECT automerge_bytes FROM spaces WHERE id = ?1",
        [space_id],
        |row| row.get(0),
    ).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(format!("Space {space_id}")),
        other => StorageError::Sqlite(other),
    })
}

pub fn update_space_doc(
    conn: &Connection,
    space_id: &str,
    automerge_bytes: &[u8],
) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE spaces SET automerge_bytes = ?1 WHERE id = ?2",
        params![automerge_bytes, space_id],
    )?;
    Ok(())
}

pub fn upsert_member(
    conn: &Connection,
    space_id: &str,
    member: &Member,
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO space_members (space_id, pubkey, display_name, avatar_blob, kicked)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            space_id,
            member.pubkey,
            member.display_name,
            member.avatar_blob,
            member.kicked as i32,
        ],
    )?;
    Ok(())
}

pub fn set_member_kicked(
    conn: &Connection,
    space_id: &str,
    pubkey: &str,
    kicked: bool,
) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE space_members SET kicked = ?1 WHERE space_id = ?2 AND pubkey = ?3",
        params![kicked as i32, space_id, pubkey],
    )?;
    Ok(())
}

pub fn add_board(conn: &Connection, space_id: &str, board_id: &str) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR IGNORE INTO space_boards (space_id, board_id) VALUES (?1, ?2)",
        params![space_id, board_id],
    )?;
    Ok(())
}

pub fn remove_board(conn: &Connection, space_id: &str, board_id: &str) -> Result<(), StorageError> {
    conn.execute(
        "DELETE FROM space_boards WHERE space_id = ?1 AND board_id = ?2",
        params![space_id, board_id],
    )?;
    Ok(())
}

pub fn insert_invite(
    conn: &Connection,
    token_hash: &str,
    token: &str,
    space_id: &str,
    expires_at: Option<i64>,
) -> Result<(), StorageError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "INSERT INTO space_invites (token_hash, token, space_id, created_at, expires_at, revoked)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)",
        params![token_hash, token, space_id, now, expires_at],
    )?;
    Ok(())
}

pub fn revoke_all_invites(conn: &Connection, space_id: &str) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE space_invites SET revoked = 1 WHERE space_id = ?1 AND revoked = 0",
        [space_id],
    )?;
    Ok(())
}

pub fn get_active_invite_token(conn: &Connection, space_id: &str) -> Result<Option<String>, StorageError> {
    match conn.query_row(
        "SELECT token FROM space_invites WHERE space_id = ?1 AND revoked = 0 LIMIT 1",
        [space_id],
        |row| row.get::<_, String>(0),
    ) {
        Ok(token) => Ok(Some(token)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StorageError::Sqlite(e)),
    }
}

pub fn check_invite_policy(
    conn: &Connection,
    metadata: &InviteMetadata,
    local_pubkey: &str,
) -> Result<(), StorageError> {
    // Joiner path: no local record to check
    if metadata.owner_pubkey != local_pubkey {
        return Ok(());
    }
    match conn.query_row(
        "SELECT revoked, expires_at FROM space_invites WHERE token_hash = ?1",
        [&metadata.token_hash],
        |row| Ok((row.get::<_, i32>(0)?, row.get::<_, Option<i64>>(1)?)),
    ) {
        Ok((revoked, expires_at)) => {
            if revoked != 0 {
                return Err(StorageError::NotFound("This invite has been revoked".into()));
            }
            if let Some(exp) = expires_at {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                if now > exp {
                    return Err(StorageError::NotFound("This invite has expired".into()));
                }
            }
            Ok(())
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            Err(StorageError::NotFound("Token not found in local records".into()))
        }
        Err(e) => Err(StorageError::Sqlite(e)),
    }
}

// ── ProfileStore ──────────────────────────────────────────────────────────────

pub fn get_profile(conn: &Connection) -> Result<Option<UserProfile>, StorageError> {
    match conn.query_row(
        "SELECT pubkey, display_name, avatar_blob, ssh_key_path FROM user_profile WHERE pk = 'local'",
        [],
        |row| Ok(UserProfile {
            pubkey: row.get(0)?,
            display_name: row.get(1)?,
            avatar_blob: row.get(2)?,
            ssh_key_path: row.get(3)?,
        }),
    ) {
        Ok(p) => Ok(Some(p)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StorageError::Sqlite(e)),
    }
}

pub fn upsert_profile(conn: &Connection, profile: &UserProfile) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO user_profile (pk, pubkey, display_name, avatar_blob, ssh_key_path)
         VALUES ('local', ?1, ?2, ?3, ?4)",
        params![profile.pubkey, profile.display_name, profile.avatar_blob, profile.ssh_key_path],
    )?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::run_migrations;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn create_and_get_space() {
        let conn = setup();
        create_space(&conn, "space-1", "My Space", "owner-pk", b"bytes").unwrap();
        let space = get_space(&conn, "space-1").unwrap();
        assert_eq!(space.name, "My Space");
        assert_eq!(space.owner_pubkey, "owner-pk");
        assert!(space.members.is_empty());
    }

    #[test]
    fn get_space_returns_not_found() {
        let conn = setup();
        assert!(matches!(
            get_space(&conn, "nonexistent"),
            Err(StorageError::NotFound(_))
        ));
    }

    #[test]
    fn upsert_member_and_list() {
        let conn = setup();
        create_space(&conn, "s1", "S", "owner", b"bytes").unwrap();
        let member = Member {
            pubkey: "pk1".into(),
            display_name: Some("Alice".into()),
            avatar_blob: None,
            kicked: false,
        };
        upsert_member(&conn, "s1", &member).unwrap();
        let space = get_space(&conn, "s1").unwrap();
        assert_eq!(space.members.len(), 1);
        assert_eq!(space.members[0].pubkey, "pk1");
    }

    #[test]
    fn add_and_remove_board() {
        let conn = setup();
        create_space(&conn, "s1", "S", "owner", b"bytes").unwrap();
        add_board(&conn, "s1", "board-abc").unwrap();
        let space = get_space(&conn, "s1").unwrap();
        assert_eq!(space.boards.len(), 1);
        remove_board(&conn, "s1", "board-abc").unwrap();
        let space = get_space(&conn, "s1").unwrap();
        assert!(space.boards.is_empty());
    }

    #[test]
    fn invite_revocation() {
        let conn = setup();
        create_space(&conn, "s1", "S", "owner-pk", b"bytes").unwrap();
        insert_invite(&conn, "hash-abc", "TOKEN_ABC", "s1", None).unwrap();
        // active invite returns the token string (not the hash)
        let active = get_active_invite_token(&conn, "s1").unwrap();
        assert_eq!(active, Some("TOKEN_ABC".into()));
        // revoke
        revoke_all_invites(&conn, "s1").unwrap();
        let active = get_active_invite_token(&conn, "s1").unwrap();
        assert!(active.is_none());
    }

    #[test]
    fn profile_upsert_replace() {
        let conn = setup();
        let p1 = UserProfile { pubkey: "pk1".into(), display_name: Some("Alice".into()), avatar_blob: None, ssh_key_path: None };
        upsert_profile(&conn, &p1).unwrap();
        let p2 = UserProfile { pubkey: "pk2".into(), display_name: Some("Bob".into()), avatar_blob: None, ssh_key_path: None };
        upsert_profile(&conn, &p2).unwrap(); // replaces
        let loaded = get_profile(&conn).unwrap().unwrap();
        assert_eq!(loaded.pubkey, "pk2");
    }

    #[test]
    fn check_invite_policy_joiner_is_noop() {
        let conn = setup();
        let meta = InviteMetadata {
            space_id: "s1".into(),
            owner_pubkey: "owner-pk".into(),
            timestamp: 0,
            token_hash: "hash".into(),
        };
        // joiner (different pubkey) → always Ok
        assert!(check_invite_policy(&conn, &meta, "joiner-pk").is_ok());
    }

    #[test]
    fn check_invite_policy_owner_revoked() {
        let conn = setup();
        create_space(&conn, "s1", "S", "owner-pk", b"bytes").unwrap();
        insert_invite(&conn, "hash-abc", "TOKEN_ABC", "s1", None).unwrap();
        revoke_all_invites(&conn, "s1").unwrap();
        let meta = InviteMetadata {
            space_id: "s1".into(),
            owner_pubkey: "owner-pk".into(),
            timestamp: 0,
            token_hash: "hash-abc".into(),
        };
        assert!(matches!(
            check_invite_policy(&conn, &meta, "owner-pk"),
            Err(StorageError::NotFound(_))
        ));
    }
}
```

- [ ] **Step 2: Register module**

In `crates/kanban-storage/src/lib.rs`, add:
```rust
pub mod space;
```

Note: `crates/kanban-storage/Cargo.toml` already has `kanban-core = { path = "../kanban-core" }` — no Cargo.toml change needed.

- [ ] **Step 3: Run tests**

```bash
cargo test -p kanban-storage -- space::tests
```
Expected: all 8 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-storage/src/space.rs crates/kanban-storage/src/lib.rs
git commit -m "feat(kanban-storage): add SpaceStore, ProfileStore, and check_invite_policy"
```

---

## Task 6: Refactor identity loading in kanban-tauri

**Files:**
- Modify: `crates/kanban-tauri/src-tauri/src/main.rs`

- [ ] **Step 1: Replace the identity-loading block in `main()`**

The existing block in `main()` (lines ~598–611):
```rust
// Load or generate identity
let key_path = data_dir.join("identity.key");
let identity = if key_path.exists() { ... } else { ... };
```

Replace it with a new helper function and updated setup:

```rust
fn load_identity(
    data_dir: &std::path::Path,
    conn: &rusqlite::Connection,
) -> Result<kanban_crypto::Identity, Box<dyn std::error::Error>> {
    use kanban_crypto::Identity;
    use kanban_storage::space as space_store;

    let key_path = data_dir.join("identity.key");

    // Step 1: read user_profile row
    if let Some(profile) = space_store::get_profile(conn)? {
        // Step 2: SSH key path set?
        if let Some(ssh_path) = &profile.ssh_key_path {
            let p = std::path::Path::new(ssh_path);
            if p.exists() {
                if let Ok(id) = kanban_crypto::import_ssh_identity(Some(p)) {
                    return Ok(id);
                }
            }
        }
        // Step 3: load from identity.key
        if key_path.exists() {
            let bytes = std::fs::read(&key_path)?;
            if bytes.len() == 32 {
                let arr: [u8; 32] = bytes.try_into().unwrap();
                return Ok(Identity::from_secret_bytes(&arr));
            }
        }
    }

    // Step 4: generate new identity
    let id = Identity::generate();
    std::fs::write(&key_path, id.to_secret_bytes())?;
    let new_profile = kanban_core::space::UserProfile {
        pubkey: id.public_key_hex(),
        display_name: None,
        avatar_blob: None,
        ssh_key_path: None,
    };
    space_store::upsert_profile(conn, &new_profile)?;
    Ok(id)
}
```

Then in the `setup` closure, replace the old identity block with:
```rust
let storage = Storage::open(&data_dir)
    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

let identity = load_identity(&data_dir, storage.conn())
    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
```

(Move `storage` creation before identity loading so the conn is available.)

- [ ] **Step 2: Build to verify no compile errors**

```bash
cd /Users/morteza/Desktop/monoes/monotask/crates/kanban-tauri
cargo build 2>&1 | grep -E "^error"
```
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/kanban-tauri/src-tauri/src/main.rs
git commit -m "refactor(kanban-tauri): use ProfileStore for 4-step identity resolution"
```

---

## Task 7: Add 14 Space Tauri commands

**Files:**
- Modify: `crates/kanban-tauri/src-tauri/src/main.rs`

- [ ] **Step 1: Add Space view types near the top of main.rs**

After the existing `#[derive(Serialize, Deserialize)]` structs, add:

```rust
#[derive(Serialize, Deserialize)]
struct SpaceSummaryView {
    id: String,
    name: String,
    member_count: usize,
}

#[derive(Serialize, Deserialize)]
struct MemberView {
    pubkey: String,
    display_name: Option<String>,
    avatar_b64: Option<String>,
    kicked: bool,
}

#[derive(Serialize, Deserialize)]
struct SpaceView {
    id: String,
    name: String,
    owner_pubkey: String,
    members: Vec<MemberView>,
    boards: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct UserProfileView {
    pubkey: String,
    display_name: Option<String>,
    avatar_b64: Option<String>,
    ssh_key_path: Option<String>,
}
```

- [ ] **Step 2: Add all 14 Space commands**

Add these functions before `fn main()`:

```rust
// ── Space helpers ─────────────────────────────────────────────────────────────

fn space_to_view(space: kanban_core::space::Space) -> SpaceView {
    SpaceView {
        id: space.id,
        name: space.name,
        owner_pubkey: space.owner_pubkey,
        members: space.members.into_iter().map(|m| MemberView {
            pubkey: m.pubkey,
            display_name: m.display_name,
            avatar_b64: m.avatar_blob.map(|b| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&b)
            }),
            kicked: m.kicked,
        }).collect(),
        boards: space.boards,
    }
}

fn local_member_profile(state: &AppState) -> kanban_core::space::MemberProfile {
    use kanban_storage::space as sp;
    let storage = state.storage.lock().unwrap();
    let profile = sp::get_profile(storage.conn()).ok().flatten();
    kanban_core::space::MemberProfile {
        display_name: profile.as_ref()
            .and_then(|p| p.display_name.clone())
            .unwrap_or_default(),
        avatar_b64: profile.as_ref()
            .and_then(|p| p.avatar_blob.as_ref())
            .map(|b| { use base64::Engine; base64::engine::general_purpose::STANDARD.encode(b) })
            .unwrap_or_default(),
        kicked: false,
    }
}

// ── Space commands ────────────────────────────────────────────────────────────

#[tauri::command]
fn create_space(name: String, state: State<AppState>) -> Result<SpaceView, String> {
    let space_id = uuid::Uuid::new_v4().to_string();
    let owner_pubkey = state.identity.public_key_hex();
    let mut doc = kanban_core::space::create_space_doc(&name, &owner_pubkey)
        .map_err(|e| e.to_string())?;
    let profile = local_member_profile(&state);
    kanban_core::space::add_member(&mut doc, &owner_pubkey, &profile)
        .map_err(|e| e.to_string())?;
    let bytes = doc.save();
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    kanban_storage::space::create_space(storage.conn(), &space_id, &name, &owner_pubkey, &bytes)
        .map_err(|e| e.to_string())?;
    // Add owner as SQL member
    let owner_member = kanban_core::space::Member {
        pubkey: owner_pubkey.clone(),
        display_name: if profile.display_name.is_empty() { None } else { Some(profile.display_name.clone()) },
        avatar_blob: if profile.avatar_b64.is_empty() { None } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(&profile.avatar_b64).ok()
        },
        kicked: false,
    };
    kanban_storage::space::upsert_member(storage.conn(), &space_id, &owner_member)
        .map_err(|e| e.to_string())?;
    let space = kanban_storage::space::get_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    Ok(space_to_view(space))
}

#[tauri::command]
fn list_spaces(state: State<AppState>) -> Result<Vec<SpaceSummaryView>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let summaries = kanban_storage::space::list_spaces(storage.conn())
        .map_err(|e| e.to_string())?;
    Ok(summaries.into_iter().map(|s| SpaceSummaryView {
        id: s.id, name: s.name, member_count: s.member_count,
    }).collect())
}

#[tauri::command]
fn get_space_cmd(space_id: String, state: State<AppState>) -> Result<SpaceView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let space = kanban_storage::space::get_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    Ok(space_to_view(space))
}

#[tauri::command]
fn generate_invite(space_id: String, state: State<AppState>) -> Result<String, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Revoke any existing active token first
    kanban_storage::space::revoke_all_invites(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let token = kanban_crypto::generate_invite_token(&space_id, &state.identity)
        .map_err(|e| e.to_string())?;
    let meta = kanban_crypto::verify_invite_token_signature(&token)
        .map_err(|e| e.to_string())?;
    kanban_storage::space::insert_invite(storage.conn(), &meta.token_hash, &token, &space_id, None)
        .map_err(|e| e.to_string())?;
    Ok(token)
}

#[tauri::command]
fn revoke_invite(space_id: String, state: State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    kanban_storage::space::revoke_all_invites(storage.conn(), &space_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn export_invite_file(space_id: String, path: String, state: State<AppState>) -> Result<(), String> {
    // Inline token generation (State<AppState> does not implement Clone, so we can't call generate_invite())
    let (token, space_name, doc_bytes) = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        // Revoke existing + generate fresh token
        kanban_storage::space::revoke_all_invites(storage.conn(), &space_id)
            .map_err(|e| e.to_string())?;
        let tok = kanban_crypto::generate_invite_token(&space_id, &state.identity)
            .map_err(|e| e.to_string())?;
        let meta = kanban_crypto::verify_invite_token_signature(&tok)
            .map_err(|e| e.to_string())?;
        kanban_storage::space::insert_invite(storage.conn(), &meta.token_hash, &tok, &space_id, None)
            .map_err(|e| e.to_string())?;
        let space = kanban_storage::space::get_space(storage.conn(), &space_id)
            .map_err(|e| e.to_string())?;
        let bytes = kanban_storage::space::load_space_doc(storage.conn(), &space_id)
            .map_err(|e| e.to_string())?;
        (tok, space.name, bytes)
    };
    use base64::Engine;
    let space_doc_b64 = base64::engine::general_purpose::STANDARD.encode(&doc_bytes);
    let payload = serde_json::json!({
        "token": token,
        "space_name": space_name,
        "space_doc": space_doc_b64,
    });
    std::fs::write(&path, serde_json::to_string_pretty(&payload).unwrap())
        .map_err(|e| e.to_string())
}

// SPEC DEVIATION: The spec says `get_invite_qr` returns a base64 PNG. This implementation
// returns the token string instead, and the UI renders the QR using qrcode.js (CDN).
// This avoids adding a Rust image-generation dependency for an MVP.
#[tauri::command]
fn get_invite_qr(space_id: String, state: State<AppState>) -> Result<String, String> {
    // Returns the active token string; UI renders QR via qrcode.js
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    kanban_storage::space::get_active_invite_token(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No active invite token".into())
}

#[tauri::command]
fn import_invite(token_or_path: String, state: State<AppState>) -> Result<SpaceView, String> {
    let local_pubkey = state.identity.public_key_hex();

    // 1. Parse token string
    let (token, space_name_hint, space_doc_bytes) = if token_or_path.ends_with(".space")
        || std::path::Path::new(&token_or_path).exists()
    {
        let content = std::fs::read_to_string(&token_or_path).map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        let tok = v["token"].as_str().unwrap_or("").to_string();
        let name = v["space_name"].as_str().unwrap_or("Shared Space").to_string();
        let doc_b64 = v["space_doc"].as_str().unwrap_or("");
        let doc_bytes = if doc_b64.is_empty() {
            None
        } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(doc_b64).ok()
        };
        (tok, name, doc_bytes)
    } else {
        (token_or_path.clone(), "Shared Space".to_string(), None)
    };

    // 2. Verify signature
    let meta = kanban_crypto::verify_invite_token_signature(&token)
        .map_err(|e| e.to_string())?;

    // 3. Check policy (owner-side only)
    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        kanban_storage::space::check_invite_policy(storage.conn(), &meta, &local_pubkey)
            .map_err(|e| e.to_string())?;

        // 4. Idempotency check
        let already = kanban_storage::space::get_space(storage.conn(), &meta.space_id);
        if let Ok(existing) = already {
            // Check if local user is a member
            if existing.members.iter().any(|m| m.pubkey == local_pubkey) {
                return Ok(space_to_view(existing));
            }
        }
    }

    // 5–8. Create or merge space
    let space_name = space_name_hint;
    let (mut doc, members_to_insert, boards_to_insert) = if let Some(bytes) = space_doc_bytes {
        let doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
        let members = kanban_core::space::list_members(&doc).map_err(|e| e.to_string())?;
        let boards = kanban_core::space::list_board_refs(&doc).map_err(|e| e.to_string())?;
        (doc, members, boards)
    } else {
        let mut doc = kanban_core::space::create_space_doc(&space_name, &meta.owner_pubkey)
            .map_err(|e| e.to_string())?;
        let empty = kanban_core::space::MemberProfile {
            display_name: String::new(), avatar_b64: String::new(), kicked: false,
        };
        kanban_core::space::add_member(&mut doc, &meta.owner_pubkey, &empty)
            .map_err(|e| e.to_string())?;
        // Include stub owner so SQL space_members row is created for them
        let stub_owner = kanban_core::space::Member {
            pubkey: meta.owner_pubkey.clone(),
            display_name: None,
            avatar_blob: None,
            kicked: false,
        };
        (doc, vec![stub_owner], vec![])
    };

    // Add local user to SpaceDoc
    let local_profile = local_member_profile(&state);
    kanban_core::space::add_member(&mut doc, &local_pubkey, &local_profile)
        .map_err(|e| e.to_string())?;
    let doc_bytes = doc.save();

    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Create space row (or skip if already exists from idempotency path)
    let _ = kanban_storage::space::create_space(
        storage.conn(), &meta.space_id, &space_name, &meta.owner_pubkey, &doc_bytes,
    );
    // Insert members from snapshot
    for m in &members_to_insert {
        let _ = kanban_storage::space::upsert_member(storage.conn(), &meta.space_id, m);
    }
    // Add local user SQL row
    let local_sql_member = kanban_core::space::Member {
        pubkey: local_pubkey,
        display_name: if local_profile.display_name.is_empty() { None } else { Some(local_profile.display_name.clone()) },
        avatar_blob: if local_profile.avatar_b64.is_empty() { None } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(&local_profile.avatar_b64).ok()
        },
        kicked: false,
    };
    let _ = kanban_storage::space::upsert_member(storage.conn(), &meta.space_id, &local_sql_member);
    // Insert boards (no FK check needed)
    for board_id in &boards_to_insert {
        let _ = kanban_storage::space::add_board(storage.conn(), &meta.space_id, board_id);
    }
    let space = kanban_storage::space::get_space(storage.conn(), &meta.space_id)
        .map_err(|e| e.to_string())?;
    Ok(space_to_view(space))
}

#[tauri::command]
fn add_board_to_space(space_id: String, board_id: String, state: State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Update SpaceDoc
    let bytes = kanban_storage::space::load_space_doc(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let mut doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
    kanban_core::space::add_board_ref(&mut doc, &board_id).map_err(|e| e.to_string())?;
    kanban_storage::space::update_space_doc(storage.conn(), &space_id, &doc.save())
        .map_err(|e| e.to_string())?;
    kanban_storage::space::add_board(storage.conn(), &space_id, &board_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_board_from_space(space_id: String, board_id: String, state: State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let bytes = kanban_storage::space::load_space_doc(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let mut doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
    kanban_core::space::remove_board_ref(&mut doc, &board_id).map_err(|e| e.to_string())?;
    kanban_storage::space::update_space_doc(storage.conn(), &space_id, &doc.save())
        .map_err(|e| e.to_string())?;
    kanban_storage::space::remove_board(storage.conn(), &space_id, &board_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn kick_member_cmd(space_id: String, pubkey: String, state: State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let bytes = kanban_storage::space::load_space_doc(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let mut doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
    kanban_core::space::kick_member(&mut doc, &pubkey).map_err(|e| e.to_string())?;
    kanban_storage::space::update_space_doc(storage.conn(), &space_id, &doc.save())
        .map_err(|e| e.to_string())?;
    kanban_storage::space::set_member_kicked(storage.conn(), &space_id, &pubkey, true)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_my_profile(state: State<AppState>) -> Result<UserProfileView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let profile = kanban_storage::space::get_profile(storage.conn())
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| kanban_core::space::UserProfile {
            pubkey: state.identity.public_key_hex(),
            display_name: None,
            avatar_blob: None,
            ssh_key_path: None,
        });
    Ok(UserProfileView {
        pubkey: profile.pubkey,
        display_name: profile.display_name,
        avatar_b64: profile.avatar_blob.map(|b| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(&b)
        }),
        ssh_key_path: profile.ssh_key_path,
    })
}

// SPEC DEVIATION: The spec defines `avatar: Option<Vec<u8>>` but this uses `avatar_b64: Option<String>`
// (base64-encoded string) because Tauri commands serialize parameters as JSON and byte arrays
// are more convenient to send from JS as base64 strings than as JSON arrays of numbers.
//
// DESIGN DECISION: This command replaces both `display_name` and `avatar` atomically.
// Passing `avatar_b64: null` intentionally clears the avatar. The UI must always send
// the current avatar back when the user only wants to update the name — it reads the
// current profile via `get_my_profile` first and re-sends `avatar_b64` unchanged.
#[tauri::command]
fn update_my_profile(
    display_name: String,
    avatar_b64: Option<String>,
    state: State<AppState>,
) -> Result<(), String> {
    use base64::Engine;
    let avatar_blob = avatar_b64.as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok());
    let pubkey = state.identity.public_key_hex();
    let new_profile = kanban_core::space::UserProfile {
        pubkey: pubkey.clone(),
        display_name: if display_name.is_empty() { None } else { Some(display_name.clone()) },
        avatar_blob: avatar_blob.clone(),
        ssh_key_path: None, // preserved from existing profile
    };
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Preserve ssh_key_path from existing profile
    let existing = kanban_storage::space::get_profile(storage.conn()).ok().flatten();
    let final_profile = kanban_core::space::UserProfile {
        ssh_key_path: existing.and_then(|p| p.ssh_key_path),
        ..new_profile
    };
    kanban_storage::space::upsert_profile(storage.conn(), &final_profile)
        .map_err(|e| e.to_string())?;
    // Propagate to all SpaceDocs
    let summaries = kanban_storage::space::list_spaces(storage.conn())
        .map_err(|e| e.to_string())?;
    let member_profile = kanban_core::space::MemberProfile {
        display_name: display_name.clone(),
        avatar_b64: avatar_b64.clone().unwrap_or_default(),
        kicked: false,
    };
    for summary in summaries {
        if let Ok(bytes) = kanban_storage::space::load_space_doc(storage.conn(), &summary.id) {
            if let Ok(mut doc) = automerge::AutoCommit::load(&bytes) {
                let _ = kanban_core::space::add_member(&mut doc, &pubkey, &member_profile);
                let _ = kanban_storage::space::update_space_doc(storage.conn(), &summary.id, &doc.save());
            }
        }
        // Update SQL cache
        let sql_member = kanban_core::space::Member {
            pubkey: pubkey.clone(),
            display_name: if display_name.is_empty() { None } else { Some(display_name.clone()) },
            avatar_blob: avatar_blob.clone(),
            kicked: false,
        };
        let _ = kanban_storage::space::upsert_member(storage.conn(), &summary.id, &sql_member);
    }
    Ok(())
}

#[tauri::command]
fn import_ssh_key(path: Option<String>, state: State<AppState>) -> Result<String, String> {
    let path_ref = path.as_deref().map(std::path::Path::new);
    let identity = kanban_crypto::import_ssh_identity(path_ref)
        .map_err(|e| e.to_string())?;
    let pubkey = identity.public_key_hex();
    let key_bytes = identity.to_secret_bytes();
    // Persist the imported key bytes (overwrite identity.key)
    std::fs::write(state.data_dir.join("identity.key"), &key_bytes)
        .map_err(|e| e.to_string())?;
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Preserve display_name and avatar_blob from existing profile — only pubkey and ssh_key_path change
    let existing = kanban_storage::space::get_profile(storage.conn()).ok().flatten();
    let updated_profile = kanban_core::space::UserProfile {
        pubkey: pubkey.clone(),
        display_name: existing.as_ref().and_then(|p| p.display_name.clone()),
        avatar_blob: existing.and_then(|p| p.avatar_blob),
        ssh_key_path: path,
    };
    kanban_storage::space::upsert_profile(storage.conn(), &updated_profile)
        .map_err(|e| e.to_string())?;
    Ok(pubkey)
}
```

**Note on `import_ssh_key`:** The identity.key write requires the `data_dir`. Add `data_dir: std::path::PathBuf` to `AppState` and set it in setup:
```rust
struct AppState {
    storage: Mutex<Storage>,
    identity: Identity,
    data_dir: std::path::PathBuf,
}
```
Then in `import_ssh_key`, write: `std::fs::write(state.data_dir.join("identity.key"), key_bytes).map_err(|e| e.to_string())?;`

- [ ] **Step 3: Register all commands in `invoke_handler`**

Add to the `tauri::generate_handler![]` macro (after existing commands):
```rust
create_space,
list_spaces,
get_space_cmd,
generate_invite,
revoke_invite,
export_invite_file,
get_invite_qr,
import_invite,
add_board_to_space,
remove_board_from_space,
kick_member_cmd,
get_my_profile,
update_my_profile,
import_ssh_key,
```

- [ ] **Step 4: Add `base64` and `uuid` to kanban-tauri deps**

In `crates/kanban-tauri/src-tauri/Cargo.toml`:
```toml
base64 = { workspace = true }
uuid   = { workspace = true }
```

- [ ] **Step 5: Build**

```bash
cd /Users/morteza/Desktop/monoes/monotask/crates/kanban-tauri
cargo build 2>&1 | grep -E "^error"
```
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add crates/kanban-tauri/src-tauri/src/main.rs crates/kanban-tauri/src-tauri/Cargo.toml
git commit -m "feat(kanban-tauri): add 14 Space commands and data_dir in AppState"
```

---

## Task 8: Add space + profile CLI subcommands

**Files:**
- Modify: `crates/kanban-cli/src/main.rs`

- [ ] **Step 1: Add Space + Profile CLI args**

Add to the Clap derive structs in `main.rs`. First, locate the existing `Commands` enum and add:

```rust
/// Manage Spaces (shared containers for boards)
Space {
    #[command(subcommand)]
    cmd: SpaceCommands,
},
/// Manage your local identity and profile
Profile {
    #[command(subcommand)]
    cmd: ProfileCommands,
},
```

Add the new subcommand enums:

```rust
#[derive(clap::Subcommand)]
enum SpaceCommands {
    /// Create a new Space
    Create { name: String },
    /// List all local Spaces
    List,
    /// Show details of a Space
    Info { space_id: String },
    #[command(subcommand_required = true)]
    Invite {
        #[command(subcommand)]
        cmd: SpaceInviteCommands,
    },
    /// Join a Space via a token or .space file
    Join { token_or_file: String },
    #[command(subcommand_required = true)]
    Boards {
        #[command(subcommand)]
        cmd: SpaceBoardsCommands,
    },
    #[command(subcommand_required = true)]
    Members {
        #[command(subcommand)]
        cmd: SpaceMembersCommands,
    },
}

#[derive(clap::Subcommand)]
enum SpaceInviteCommands {
    /// Generate a new invite token for a Space
    Generate { space_id: String },
    /// Export an invite as a .space file
    Export { space_id: String, output_file: String },
    /// Revoke all active invites for a Space
    Revoke { space_id: String },
}

#[derive(clap::Subcommand)]
enum SpaceBoardsCommands {
    /// Add a board to a Space
    Add { space_id: String, board_id: String },
    /// Remove a board from a Space
    Remove { space_id: String, board_id: String },
    /// List boards in a Space
    List { space_id: String },
}

#[derive(clap::Subcommand)]
enum SpaceMembersCommands {
    /// List members of a Space
    List { space_id: String },
    /// Kick a member from a Space
    Kick { space_id: String, pubkey: String },
}

#[derive(clap::Subcommand)]
enum ProfileCommands {
    /// Show your current profile
    Show,
    /// Set your display name
    SetName { name: String },
    /// Set your avatar from an image file
    SetAvatar { path: String },
    /// Import an SSH Ed25519 key as your identity
    ImportSshKey { path: Option<String> },
}
```

- [ ] **Step 2: Add command dispatch in main()**

In the `match` block that dispatches commands, add (the CLI already has `let dir = data_dir(&cli)?;` — pass it through):

```rust
Commands::Space { cmd } => handle_space(cmd, &mut storage, &identity),
Commands::Profile { cmd } => handle_profile(cmd, &mut storage, &identity, &dir),
```

- [ ] **Step 3: Implement handle_space and handle_profile**

```rust
fn handle_space(cmd: SpaceCommands, storage: &mut Storage, identity: &Identity) -> anyhow::Result<()> {
    use kanban_core::space as cs;
    use kanban_storage::space as ss;

    match cmd {
        SpaceCommands::Create { name } => {
            let space_id = uuid::Uuid::new_v4().to_string();
            let owner_pubkey = identity.public_key_hex();
            let mut doc = cs::create_space_doc(&name, &owner_pubkey)?;
            let profile = get_local_member_profile(storage.conn());
            cs::add_member(&mut doc, &owner_pubkey, &profile)?;
            let bytes = doc.save();
            ss::create_space(storage.conn(), &space_id, &name, &owner_pubkey, &bytes)?;
            let owner_member = cs::Member {
                pubkey: owner_pubkey.clone(),
                display_name: if profile.display_name.is_empty() { None } else { Some(profile.display_name.clone()) },
                avatar_blob: None,
                kicked: false,
            };
            ss::upsert_member(storage.conn(), &space_id, &owner_member)?;
            println!("Created Space: {} ({})", name, space_id);
        }
        SpaceCommands::List => {
            let spaces = ss::list_spaces(storage.conn())?;
            if spaces.is_empty() {
                println!("No spaces found.");
            } else {
                for s in spaces {
                    println!("{} | {} | {} members", s.id, s.name, s.member_count);
                }
            }
        }
        SpaceCommands::Info { space_id } => {
            let space = ss::get_space(storage.conn(), &space_id)?;
            println!("Space: {} ({})", space.name, space.id);
            println!("Owner: {}", space.owner_pubkey);
            println!("Members ({}):", space.members.len());
            for m in &space.members {
                let name = m.display_name.as_deref().unwrap_or("(unnamed)");
                let kicked = if m.kicked { " [kicked]" } else { "" };
                println!("  {}  {}{}", &m.pubkey[..16], name, kicked);
            }
            println!("Boards ({}):", space.boards.len());
            for b in &space.boards {
                println!("  {}", b);
            }
        }
        SpaceCommands::Invite { cmd } => match cmd {
            SpaceInviteCommands::Generate { space_id } => {
                ss::revoke_all_invites(storage.conn(), &space_id)?;
                let token = kanban_crypto::generate_invite_token(&space_id, identity)?;
                let meta = kanban_crypto::verify_invite_token_signature(&token)?;
                ss::insert_invite(storage.conn(), &meta.token_hash, &token, &space_id, None)?;
                println!("{}", token);
            }
            SpaceInviteCommands::Export { space_id, output_file } => {
                ss::revoke_all_invites(storage.conn(), &space_id)?;
                let token = kanban_crypto::generate_invite_token(&space_id, identity)?;
                let meta = kanban_crypto::verify_invite_token_signature(&token)?;
                ss::insert_invite(storage.conn(), &meta.token_hash, &token, &space_id, None)?;
                let space = ss::get_space(storage.conn(), &space_id)?;
                let doc_bytes = ss::load_space_doc(storage.conn(), &space_id)?;
                use base64::Engine;
                let space_doc_b64 = base64::engine::general_purpose::STANDARD.encode(&doc_bytes);
                let payload = serde_json::json!({
                    "token": token,
                    "space_name": space.name,
                    "space_doc": space_doc_b64,
                });
                std::fs::write(&output_file, serde_json::to_string_pretty(&payload)?)?;
                println!("Exported invite to {}", output_file);
            }
            SpaceInviteCommands::Revoke { space_id } => {
                ss::revoke_all_invites(storage.conn(), &space_id)?;
                println!("Revoked all active invites for {}", space_id);
            }
        },
        SpaceCommands::Join { token_or_file } => {
            let local_pubkey = identity.public_key_hex();
            let (token, space_name, doc_bytes_opt) =
                parse_token_or_file(&token_or_file)?;
            let meta = kanban_crypto::verify_invite_token_signature(&token)?;
            ss::check_invite_policy(storage.conn(), &meta, &local_pubkey)?;
            // Idempotency
            if let Ok(existing) = ss::get_space(storage.conn(), &meta.space_id) {
                if existing.members.iter().any(|m| m.pubkey == local_pubkey) {
                    println!("Already a member of Space: {} ({})", existing.name, meta.space_id);
                    return Ok(());
                }
            }
            let local_profile = get_local_member_profile(storage.conn());
            let (mut doc, members, boards) = if let Some(bytes) = doc_bytes_opt {
                let doc = automerge::AutoCommit::load(&bytes)?;
                let members = cs::list_members(&doc)?;
                let boards = cs::list_board_refs(&doc)?;
                (doc, members, boards)
            } else {
                let mut doc = cs::create_space_doc(&space_name, &meta.owner_pubkey)?;
                let empty = cs::MemberProfile { display_name: String::new(), avatar_b64: String::new(), kicked: false };
                cs::add_member(&mut doc, &meta.owner_pubkey, &empty)?;
                // Include stub owner so SQL space_members row is created for them
                let stub_owner = cs::Member {
                    pubkey: meta.owner_pubkey.clone(),
                    display_name: None,
                    avatar_blob: None,
                    kicked: false,
                };
                (doc, vec![stub_owner], vec![])
            };
            cs::add_member(&mut doc, &local_pubkey, &local_profile)?;
            let doc_bytes = doc.save();
            let _ = ss::create_space(storage.conn(), &meta.space_id, &space_name, &meta.owner_pubkey, &doc_bytes);
            for m in &members {
                let _ = ss::upsert_member(storage.conn(), &meta.space_id, m);
            }
            let local_sql = cs::Member {
                pubkey: local_pubkey,
                display_name: if local_profile.display_name.is_empty() { None } else { Some(local_profile.display_name) },
                avatar_blob: None,
                kicked: false,
            };
            ss::upsert_member(storage.conn(), &meta.space_id, &local_sql)?;
            for b in &boards {
                let _ = ss::add_board(storage.conn(), &meta.space_id, b);
            }
            println!("Joined Space: {} ({})", space_name, meta.space_id);
        }
        SpaceCommands::Boards { cmd } => match cmd {
            SpaceBoardsCommands::Add { space_id, board_id } => {
                let bytes = ss::load_space_doc(storage.conn(), &space_id)?;
                let mut doc = automerge::AutoCommit::load(&bytes)?;
                cs::add_board_ref(&mut doc, &board_id)?;
                ss::update_space_doc(storage.conn(), &space_id, &doc.save())?;
                ss::add_board(storage.conn(), &space_id, &board_id)?;
                println!("Added board {} to Space {}", board_id, space_id);
            }
            SpaceBoardsCommands::Remove { space_id, board_id } => {
                let bytes = ss::load_space_doc(storage.conn(), &space_id)?;
                let mut doc = automerge::AutoCommit::load(&bytes)?;
                cs::remove_board_ref(&mut doc, &board_id)?;
                ss::update_space_doc(storage.conn(), &space_id, &doc.save())?;
                ss::remove_board(storage.conn(), &space_id, &board_id)?;
                println!("Removed board {} from Space {}", board_id, space_id);
            }
            SpaceBoardsCommands::List { space_id } => {
                let space = ss::get_space(storage.conn(), &space_id)?;
                for b in &space.boards { println!("{}", b); }
            }
        },
        SpaceCommands::Members { cmd } => match cmd {
            SpaceMembersCommands::List { space_id } => {
                let space = ss::get_space(storage.conn(), &space_id)?;
                for m in &space.members {
                    let name = m.display_name.as_deref().unwrap_or("(unnamed)");
                    let kicked = if m.kicked { " [kicked]" } else { "" };
                    println!("{}  {}{}", m.pubkey, name, kicked);
                }
            }
            SpaceMembersCommands::Kick { space_id, pubkey } => {
                let bytes = ss::load_space_doc(storage.conn(), &space_id)?;
                let mut doc = automerge::AutoCommit::load(&bytes)?;
                cs::kick_member(&mut doc, &pubkey)?;
                ss::update_space_doc(storage.conn(), &space_id, &doc.save())?;
                ss::set_member_kicked(storage.conn(), &space_id, &pubkey, true)?;
                println!("Kicked {} from Space {}", pubkey, space_id);
            }
        },
    }
    Ok(())
}

fn handle_profile(cmd: ProfileCommands, storage: &mut Storage, identity: &Identity, data_dir: &std::path::Path) -> anyhow::Result<()> {
    use kanban_storage::space as ss;

    match cmd {
        ProfileCommands::Show => {
            let profile = ss::get_profile(storage.conn())?
                .unwrap_or_else(|| kanban_core::space::UserProfile {
                    pubkey: identity.public_key_hex(),
                    display_name: None,
                    avatar_blob: None,
                    ssh_key_path: None,
                });
            println!("Pubkey:       {}", profile.pubkey);
            println!("Display name: {}", profile.display_name.as_deref().unwrap_or("(not set)"));
            println!("Avatar:       {}", if profile.avatar_blob.is_some() { "set" } else { "not set" });
            println!("SSH key path: {}", profile.ssh_key_path.as_deref().unwrap_or("(auto-generated)"));
        }
        ProfileCommands::SetName { name } => {
            let existing = ss::get_profile(storage.conn())?.unwrap_or_else(|| kanban_core::space::UserProfile {
                pubkey: identity.public_key_hex(),
                display_name: None,
                avatar_blob: None,
                ssh_key_path: None,
            });
            ss::upsert_profile(storage.conn(), &kanban_core::space::UserProfile {
                display_name: Some(name.clone()),
                ..existing
            })?;
            println!("Display name set to: {}", name);
        }
        ProfileCommands::SetAvatar { path } => {
            let avatar_blob = std::fs::read(&path)?;
            let existing = ss::get_profile(storage.conn())?.unwrap_or_else(|| kanban_core::space::UserProfile {
                pubkey: identity.public_key_hex(),
                display_name: None,
                avatar_blob: None,
                ssh_key_path: None,
            });
            ss::upsert_profile(storage.conn(), &kanban_core::space::UserProfile {
                avatar_blob: Some(avatar_blob),
                ..existing
            })?;
            println!("Avatar set from {}", path);
        }
        ProfileCommands::ImportSshKey { path } => {
            let path_ref = path.as_deref().map(std::path::Path::new);
            let new_identity = kanban_crypto::import_ssh_identity(path_ref)?;
            let pubkey = new_identity.public_key_hex();
            let key_bytes = new_identity.to_secret_bytes();
            // data_dir is passed in from main() — the CLI already has `let dir = data_dir(&cli)?;`
            std::fs::write(data_dir.join("identity.key"), key_bytes)?;
            let existing = ss::get_profile(storage.conn())?;
            ss::upsert_profile(storage.conn(), &kanban_core::space::UserProfile {
                pubkey: pubkey.clone(),
                display_name: existing.as_ref().and_then(|p| p.display_name.clone()),
                avatar_blob: existing.and_then(|p| p.avatar_blob),
                ssh_key_path: path,
            })?;
            println!("Imported SSH key. New pubkey: {}", pubkey);
        }
    }
    Ok(())
}

fn get_local_member_profile(conn: &rusqlite::Connection) -> kanban_core::space::MemberProfile {
    use kanban_storage::space as ss;
    let profile = ss::get_profile(conn).ok().flatten();
    kanban_core::space::MemberProfile {
        display_name: profile.as_ref()
            .and_then(|p| p.display_name.clone())
            .unwrap_or_default(),
        avatar_b64: profile.as_ref()
            .and_then(|p| p.avatar_blob.as_ref())
            .map(|b| { use base64::Engine; base64::engine::general_purpose::STANDARD.encode(b) })
            .unwrap_or_default(),
        kicked: false,
    }
}

fn parse_token_or_file(input: &str) -> anyhow::Result<(String, String, Option<Vec<u8>>)> {
    if input.ends_with(".space") || std::path::Path::new(input).exists() {
        let content = std::fs::read_to_string(input)?;
        let v: serde_json::Value = serde_json::from_str(&content)?;
        let token = v["token"].as_str().unwrap_or("").to_string();
        let name = v["space_name"].as_str().unwrap_or("Shared Space").to_string();
        let doc_b64 = v["space_doc"].as_str().unwrap_or("");
        let doc_bytes = if doc_b64.is_empty() {
            None
        } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(doc_b64).ok()
        };
        Ok((token, name, doc_bytes))
    } else {
        Ok((input.to_string(), "Shared Space".to_string(), None))
    }
}
```

**CLI Cargo.toml changes:** The existing `crates/kanban-cli/Cargo.toml` already has `kanban-core`, `kanban-storage`, `kanban-crypto`. Add these three new direct deps:

```toml
automerge = { workspace = true }
uuid      = { workspace = true }
base64    = { workspace = true }
```

Note: `automerge` is used directly (e.g. `automerge::AutoCommit::load`), so it must be an explicit dep even though it's also pulled in transitively via `kanban-storage`.

- [ ] **Step 4: Build CLI**

```bash
cd /Users/morteza/Desktop/monoes/monotask
cargo build -p kanban-cli 2>&1 | grep -E "^error"
```
Expected: no errors.

- [ ] **Step 5: Smoke test CLI space commands**

```bash
# From the monotask root
KANBAN_DATA=/tmp/space-test-cli cargo run -p kanban-cli --bin app-cli -- space create "My Team"
KANBAN_DATA=/tmp/space-test-cli cargo run -p kanban-cli --bin app-cli -- space list
KANBAN_DATA=/tmp/space-test-cli cargo run -p kanban-cli --bin app-cli -- space invite generate <space-id-from-above>
KANBAN_DATA=/tmp/space-test-cli cargo run -p kanban-cli --bin app-cli -- profile show
KANBAN_DATA=/tmp/space-test-cli cargo run -p kanban-cli --bin app-cli -- profile set-name "Alice"
KANBAN_DATA=/tmp/space-test-cli cargo run -p kanban-cli --bin app-cli -- profile show
```

Expected: each command succeeds without error, space + profile data shown.

- [ ] **Step 6: Commit**

```bash
git add crates/kanban-cli/src/main.rs crates/kanban-cli/Cargo.toml
git commit -m "feat(kanban-cli): add space and profile subcommands"
```

---

## Task 9: Space UI in index.html

**Files:**
- Modify: `crates/kanban-tauri/src/index.html`

- [ ] **Step 1: Add qrcode.js script tag**

In the `<head>`, add after existing font/style imports:
```html
<script src="https://cdnjs.cloudflare.com/ajax/libs/qrcodejs/1.0.0/qrcode.min.js"></script>
```

- [ ] **Step 2: Add Spaces sidebar panel HTML**

Inside the main layout, add a left sidebar. Before the existing `#board-dashboard` div, add:

```html
<!-- Spaces sidebar -->
<div id="spaces-sidebar">
  <div class="sidebar-header">
    <span>Spaces</span>
    <button id="btn-new-space" title="New Space">+</button>
  </div>
  <ul id="spaces-list"></ul>
  <div class="sidebar-footer">
    <button id="btn-profile" title="Profile settings">⚙ Profile</button>
  </div>
</div>
```

- [ ] **Step 3: Add Space detail panel HTML**

After `#board-dashboard`, add:

```html
<div id="space-panel" style="display:none;">
  <div class="space-panel-header">
    <button id="btn-back-to-boards">← Boards</button>
    <h2 id="space-panel-name"></h2>
  </div>
  <div class="space-tabs">
    <button class="space-tab active" data-tab="members">Members</button>
    <button class="space-tab" data-tab="boards">Boards</button>
    <button class="space-tab" data-tab="invite">Invite</button>
  </div>

  <!-- Members tab -->
  <div id="tab-members" class="space-tab-content">
    <ul id="space-members-list"></ul>
  </div>

  <!-- Boards tab -->
  <div id="tab-boards" class="space-tab-content" style="display:none;">
    <ul id="space-boards-list"></ul>
    <div class="add-board-row">
      <select id="select-board-to-add"></select>
      <button id="btn-add-board-to-space">Add Board</button>
    </div>
  </div>

  <!-- Invite tab -->
  <div id="tab-invite" class="space-tab-content" style="display:none;">
    <div id="invite-token-section" style="display:none;">
      <label>Invite Token</label>
      <div class="token-row">
        <input id="invite-token-input" readonly style="width:100%;max-width:450px;" />
        <button id="btn-copy-token">Copy</button>
      </div>
      <div id="invite-qr"></div>
      <div class="invite-actions">
        <button id="btn-export-invite">Export .space file</button>
        <button id="btn-revoke-invite">Revoke &amp; Regenerate</button>
      </div>
    </div>
    <div id="no-invite-section">
      <button id="btn-generate-invite">Generate Invite</button>
    </div>
  </div>
</div>

<!-- Create Space modal -->
<div id="modal-create-space" class="modal" style="display:none;">
  <div class="modal-box">
    <h3>New Space</h3>
    <input id="input-space-name" placeholder="Space name" />
    <div class="modal-actions">
      <button id="btn-create-space-confirm">Create</button>
      <button id="btn-create-space-cancel">Cancel</button>
    </div>
  </div>
</div>

<!-- Profile settings modal -->
<div id="modal-profile" class="modal" style="display:none;">
  <div class="modal-box">
    <h3>Profile</h3>
    <div class="profile-field">
      <label>Display Name</label>
      <input id="input-display-name" placeholder="Your name" />
    </div>
    <div class="profile-field">
      <label>Public Key</label>
      <div class="pubkey-row">
        <span id="profile-pubkey-short"></span>
        <button id="btn-copy-pubkey">Copy Full Key</button>
      </div>
    </div>
    <div class="profile-field">
      <label>SSH Key Path (optional)</label>
      <input id="input-ssh-key-path" placeholder="~/.ssh/id_ed25519" />
      <button id="btn-import-ssh">Import</button>
    </div>
    <div class="modal-actions">
      <button id="btn-save-profile">Save</button>
      <button id="btn-close-profile">Close</button>
    </div>
  </div>
</div>
```

- [ ] **Step 4: Add CSS for spaces sidebar and panels**

In the `<style>` block, add:

```css
/* Layout */
body { display: flex; flex-direction: row; }
#spaces-sidebar {
  width: 200px; min-width: 160px;
  background: #0d0d1a;
  border-right: 1px solid #1e1e3a;
  display: flex; flex-direction: column;
  padding: 12px 8px;
}
.sidebar-header {
  display: flex; justify-content: space-between; align-items: center;
  color: #c8962a; font-size: 12px; text-transform: uppercase;
  letter-spacing: 0.1em; margin-bottom: 8px;
}
#spaces-list { list-style: none; padding: 0; margin: 0; flex: 1; }
#spaces-list li {
  padding: 6px 10px; cursor: pointer; border-radius: 4px;
  color: #888; font-size: 13px;
}
#spaces-list li:hover, #spaces-list li.active { color: #eee; background: #1a1a2e; }
.sidebar-footer { border-top: 1px solid #1e1e3a; padding-top: 8px; }
.sidebar-footer button { width: 100%; background: none; border: none; color: #555; cursor: pointer; font-size: 12px; text-align: left; }
.sidebar-footer button:hover { color: #c8962a; }

/* Space panel */
#space-panel { flex: 1; padding: 24px; overflow-y: auto; }
.space-panel-header { display: flex; align-items: center; gap: 16px; margin-bottom: 16px; }
.space-tabs { display: flex; gap: 8px; margin-bottom: 16px; border-bottom: 1px solid #1e1e3a; }
.space-tab { background: none; border: none; color: #555; cursor: pointer; padding: 8px 16px; font-size: 13px; }
.space-tab.active { color: #c8962a; border-bottom: 2px solid #c8962a; }
.space-tab-content { padding: 12px 0; }

/* Members list */
#space-members-list { list-style: none; padding: 0; }
#space-members-list li {
  display: flex; align-items: center; gap: 12px;
  padding: 8px 0; border-bottom: 1px solid #1a1a2e; font-size: 13px;
}
.member-avatar {
  width: 32px; height: 32px; border-radius: 50%;
  background: #1e1e3a; display: flex; align-items: center; justify-content: center;
  color: #c8962a; font-weight: bold; font-size: 14px;
}
.member-info { flex: 1; }
.member-kicked { opacity: 0.4; }
.btn-kick { background: none; border: 1px solid #3a1a1a; color: #c05050; cursor: pointer; padding: 2px 8px; border-radius: 3px; font-size: 11px; }

/* Boards tab */
#space-boards-list { list-style: none; padding: 0; }
#space-boards-list li { display: flex; justify-content: space-between; align-items: center; padding: 6px 0; font-size: 13px; }
.add-board-row { display: flex; gap: 8px; margin-top: 12px; }

/* Invite tab */
.token-row { display: flex; gap: 8px; margin-bottom: 12px; }
.invite-actions { display: flex; gap: 8px; margin-top: 8px; }
#invite-qr { margin: 12px 0; }

/* Space badge on board cards */
.space-badge {
  display: inline-block; background: #1e2a3a; color: #3870c0;
  border: 1px solid #3870c0; border-radius: 3px;
  padding: 1px 6px; font-size: 10px; margin-left: 4px;
}

/* Profile modal */
.profile-field { margin-bottom: 12px; }
.profile-field label { display: block; color: #888; font-size: 11px; margin-bottom: 4px; }
.pubkey-row { display: flex; align-items: center; gap: 8px; font-family: monospace; font-size: 12px; color: #555; }
```

- [ ] **Step 5: Add JavaScript for Spaces**

In the `<script>` block, add Space-related JS:

```javascript
// ── Space state ───────────────────────────────────────────────────────────────
let currentSpaceId = null;
let currentSpaceData = null;
let allBoards = []; // cached for "add board" dropdown

// ── Sidebar ───────────────────────────────────────────────────────────────────
async function loadSpacesSidebar() {
  const spaces = await invoke('list_spaces');
  const list = document.getElementById('spaces-list');
  list.innerHTML = '';
  for (const s of spaces) {
    const li = document.createElement('li');
    li.textContent = s.name;
    li.dataset.spaceId = s.id;
    li.title = `${s.member_count} member(s)`;
    if (s.id === currentSpaceId) li.classList.add('active');
    li.onclick = () => openSpacePanel(s.id);
    list.appendChild(li);
  }
}

async function openSpacePanel(spaceId) {
  currentSpaceId = spaceId;
  document.getElementById('board-dashboard').style.display = 'none';
  document.getElementById('board-view').style.display = 'none';
  document.getElementById('space-panel').style.display = 'block';
  await refreshSpacePanel();
  await loadSpacesSidebar(); // highlight active
}

async function refreshSpacePanel() {
  const space = await invoke('get_space_cmd', { spaceId: currentSpaceId });
  currentSpaceData = space;
  document.getElementById('space-panel-name').textContent = space.name;
  renderMembersTab(space);
  renderBoardsTab(space);
  await renderInviteTab();
}

function renderMembersTab(space) {
  const myPubkey = window._myPubkey || '';
  const isOwner = space.owner_pubkey === myPubkey;
  const ul = document.getElementById('space-members-list');
  ul.innerHTML = '';
  for (const m of space.members) {
    const li = document.createElement('li');
    if (m.kicked) li.classList.add('member-kicked');
    const initial = (m.display_name || m.pubkey).charAt(0).toUpperCase();
    li.innerHTML = `
      <div class="member-avatar">${initial}</div>
      <div class="member-info">
        <div>${m.display_name || '(unnamed)'}</div>
        <div style="font-size:11px;color:#444;">${m.pubkey.slice(0, 16)}…</div>
      </div>
      ${isOwner && !m.kicked && m.pubkey !== myPubkey
        ? `<button class="btn-kick" data-pubkey="${m.pubkey}">Kick</button>`
        : ''}
    `;
    ul.appendChild(li);
  }
  ul.querySelectorAll('.btn-kick').forEach(btn => {
    btn.onclick = async () => {
      if (!confirm(`Kick ${btn.dataset.pubkey.slice(0,16)}…?`)) return;
      await invoke('kick_member_cmd', { spaceId: currentSpaceId, pubkey: btn.dataset.pubkey });
      await refreshSpacePanel();
    };
  });
}

async function renderBoardsTab(space) {
  const ul = document.getElementById('space-boards-list');
  ul.innerHTML = '';
  for (const boardId of space.boards) {
    const li = document.createElement('li');
    const board = allBoards.find(b => b.id === boardId);
    li.innerHTML = `
      <span>${board ? board.title : boardId}</span>
      <button class="btn-remove-board" data-board="${boardId}">Remove</button>
    `;
    ul.appendChild(li);
  }
  ul.querySelectorAll('.btn-remove-board').forEach(btn => {
    btn.onclick = async () => {
      await invoke('remove_board_from_space', { spaceId: currentSpaceId, boardId: btn.dataset.board });
      await refreshSpacePanel();
    };
  });
  // Populate "add board" dropdown with boards NOT already in space
  const sel = document.getElementById('select-board-to-add');
  sel.innerHTML = '<option value="">Select board…</option>';
  for (const b of allBoards) {
    if (!space.boards.includes(b.id)) {
      const opt = document.createElement('option');
      opt.value = b.id;
      opt.textContent = b.title;
      sel.appendChild(opt);
    }
  }
}

async function renderInviteTab() {
  const token = await invoke('get_invite_qr', { spaceId: currentSpaceId }).catch(() => null);
  const tokenSection = document.getElementById('invite-token-section');
  const noInviteSection = document.getElementById('no-invite-section');
  if (token) {
    tokenSection.style.display = 'block';
    noInviteSection.style.display = 'none';
    document.getElementById('invite-token-input').value = token;
    const qrDiv = document.getElementById('invite-qr');
    qrDiv.innerHTML = '';
    new QRCode(qrDiv, { text: token, width: 160, height: 160 });
  } else {
    tokenSection.style.display = 'none';
    noInviteSection.style.display = 'block';
  }
}

// ── Space tab switching ───────────────────────────────────────────────────────
document.querySelectorAll('.space-tab').forEach(tab => {
  tab.onclick = () => {
    document.querySelectorAll('.space-tab').forEach(t => t.classList.remove('active'));
    document.querySelectorAll('.space-tab-content').forEach(c => c.style.display = 'none');
    tab.classList.add('active');
    document.getElementById('tab-' + tab.dataset.tab).style.display = 'block';
  };
});

// ── New Space modal ───────────────────────────────────────────────────────────
document.getElementById('btn-new-space').onclick = () => {
  document.getElementById('modal-create-space').style.display = 'flex';
  document.getElementById('input-space-name').focus();
};
document.getElementById('btn-create-space-cancel').onclick = () => {
  document.getElementById('modal-create-space').style.display = 'none';
};
document.getElementById('btn-create-space-confirm').onclick = async () => {
  const name = document.getElementById('input-space-name').value.trim();
  if (!name) return;
  await invoke('create_space', { name });
  document.getElementById('modal-create-space').style.display = 'none';
  document.getElementById('input-space-name').value = '';
  await loadSpacesSidebar();
};

// ── Invite tab actions ────────────────────────────────────────────────────────
document.getElementById('btn-generate-invite').onclick = async () => {
  await invoke('generate_invite', { spaceId: currentSpaceId });
  await renderInviteTab();
};
document.getElementById('btn-copy-token').onclick = () => {
  navigator.clipboard.writeText(document.getElementById('invite-token-input').value);
};
document.getElementById('btn-revoke-invite').onclick = async () => {
  await invoke('generate_invite', { spaceId: currentSpaceId }); // implicitly revokes
  await renderInviteTab();
};
document.getElementById('btn-export-invite').onclick = async () => {
  const path = prompt('Save .space file to path:');
  if (!path) return;
  await invoke('export_invite_file', { spaceId: currentSpaceId, path });
  alert('Exported!');
};

// ── Add board to space ────────────────────────────────────────────────────────
document.getElementById('btn-add-board-to-space').onclick = async () => {
  const boardId = document.getElementById('select-board-to-add').value;
  if (!boardId) return;
  await invoke('add_board_to_space', { spaceId: currentSpaceId, boardId });
  await refreshSpacePanel();
};

// ── Back to boards ────────────────────────────────────────────────────────────
document.getElementById('btn-back-to-boards').onclick = () => {
  currentSpaceId = null;
  document.getElementById('space-panel').style.display = 'none';
  document.getElementById('board-dashboard').style.display = 'block';
  loadSpacesSidebar();
};

// ── Profile modal ─────────────────────────────────────────────────────────────
document.getElementById('btn-profile').onclick = async () => {
  const profile = await invoke('get_my_profile');
  window._myPubkey = profile.pubkey;
  document.getElementById('input-display-name').value = profile.display_name || '';
  document.getElementById('profile-pubkey-short').textContent = profile.pubkey.slice(0, 24) + '…';
  document.getElementById('modal-profile').style.display = 'flex';
};
document.getElementById('btn-copy-pubkey').onclick = async () => {
  const profile = await invoke('get_my_profile');
  navigator.clipboard.writeText(profile.pubkey);
};
document.getElementById('btn-import-ssh').onclick = async () => {
  const path = document.getElementById('input-ssh-key-path').value.trim() || null;
  const pubkey = await invoke('import_ssh_key', { path }).catch(e => { alert(e); return null; });
  if (pubkey) {
    document.getElementById('profile-pubkey-short').textContent = pubkey.slice(0, 24) + '…';
    alert('SSH key imported. New pubkey: ' + pubkey.slice(0, 16) + '…');
  }
};
document.getElementById('btn-save-profile').onclick = async () => {
  const displayName = document.getElementById('input-display-name').value.trim();
  // Re-read existing profile to preserve avatar (update_my_profile replaces both fields atomically)
  const existing = await invoke('get_my_profile').catch(() => null);
  await invoke('update_my_profile', {
    displayName,
    avatarB64: existing?.avatar_b64 ?? null,
  });
  document.getElementById('modal-profile').style.display = 'none';
};
document.getElementById('btn-close-profile').onclick = () => {
  document.getElementById('modal-profile').style.display = 'none';
};

// ── Space badges on board cards ───────────────────────────────────────────────
// Fetches all spaces and their board lists, then decorates board card DOM nodes
// with "[SpaceName]" badges. Called after board list renders.
async function addSpaceBadgesToBoards() {
  const spaces = await invoke('list_spaces').catch(() => []);
  if (!spaces.length) return;
  // Build a map: boardId → [spaceName, ...]
  const boardSpaceMap = {};
  for (const summary of spaces) {
    const full = await invoke('get_space_cmd', { spaceId: summary.id }).catch(() => null);
    if (!full) continue;
    for (const boardId of full.boards) {
      if (!boardSpaceMap[boardId]) boardSpaceMap[boardId] = [];
      boardSpaceMap[boardId].push(summary.name);
    }
  }
  // Annotate board card elements (assumes each board card has data-board-id attribute)
  document.querySelectorAll('[data-board-id]').forEach(card => {
    const boardId = card.dataset.boardId;
    const spaceNames = boardSpaceMap[boardId];
    if (!spaceNames?.length) return;
    const existing = card.querySelector('.space-badges');
    if (existing) existing.remove();
    const badges = document.createElement('span');
    badges.className = 'space-badges';
    badges.style.cssText = 'font-size:0.7em;color:#888;margin-left:6px;';
    badges.textContent = spaceNames.map(n => `[${n}]`).join(' ');
    card.appendChild(badges);
  });
}

// ── Init: load my pubkey + spaces ─────────────────────────────────────────────
async function initSpaces() {
  const profile = await invoke('get_my_profile').catch(() => null);
  if (profile) window._myPubkey = profile.pubkey;
  await loadSpacesSidebar();
  await addSpaceBadgesToBoards(); // decorate any already-rendered board cards
}

// Call initSpaces() when the app loads (add to existing init/DOMContentLoaded)
```

At the bottom of the existing `DOMContentLoaded` or init call, add:
```javascript
initSpaces();
```

Also call `addSpaceBadgesToBoards()` after the board list re-renders (find the existing board-list render function in `index.html` and add a call at its end).

- [ ] **Step 6: Build Tauri app**

```bash
cd /Users/morteza/Desktop/monoes/monotask/crates/kanban-tauri
cargo tauri dev 2>&1 | head -40
```
Expected: app opens without errors. Spaces sidebar visible on left.

- [ ] **Step 7: Manual smoke test**

1. Click `+` in Spaces sidebar → create "Test Space"
2. Navigate to Space detail → Members tab shows you as the owner
3. Boards tab → add a board
4. Invite tab → click "Generate Invite" → token appears + QR renders
5. Click "Copy" → token in clipboard
6. Click "Revoke & Regenerate" → new token and QR appear
7. Click "Export .space file" → file saved
8. Profile → ⚙ Profile → set display name → Save
9. Click "← Boards" → back to board dashboard

- [ ] **Step 8: Commit**

```bash
git add crates/kanban-tauri/src/index.html
git commit -m "feat(kanban-tauri): add Space UI (sidebar, detail panel, invite tab, profile modal)"
```

---

## Final: Run all tests

- [ ] **Run full test suite**

```bash
cd /Users/morteza/Desktop/monoes/monotask
cargo test --workspace 2>&1 | tail -20
```
Expected: all tests pass, 0 failures.

- [ ] **CLI integration smoke test**

```bash
KANBAN_DATA=/tmp/final-test cargo run -p kanban-cli --bin app-cli -- space create "Alpha"
SPACE_ID=$(KANBAN_DATA=/tmp/final-test cargo run -p kanban-cli --bin app-cli -- space list | awk '{print $1}')
KANBAN_DATA=/tmp/final-test cargo run -p kanban-cli --bin app-cli -- space invite generate $SPACE_ID
KANBAN_DATA=/tmp/final-test cargo run -p kanban-cli --bin app-cli -- space invite export $SPACE_ID /tmp/alpha.space
KANBAN_DATA=/tmp/final-test cargo run -p kanban-cli --bin app-cli -- profile set-name "Alice"
KANBAN_DATA=/tmp/final-test cargo run -p kanban-cli --bin app-cli -- profile show
```
Expected: all commands succeed without error.
