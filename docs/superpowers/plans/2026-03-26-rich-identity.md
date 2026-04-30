# Rich Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expand user profiles with photo, bio, role, accent color, and presence; show human-readable identity everywhere raw IDs currently appear.

**Architecture:** New profile fields flow from `user_profile` SQLite → `UserProfile` struct → `MemberProfile` CRDT → space doc sync → peers. A new `NetCommand::GetPeerPubkeys` exposes the swarm's `pubkey_cache` so `get_sync_info_cmd` can cross-reference connected peers with member profiles.

**Tech Stack:** Rust (automerge, rusqlite, libp2p, Tauri v2), vanilla JS/HTML

---

## File Map

| File | Change |
|------|--------|
| `crates/kanban-storage/src/schema.rs` | Add `run_migrations_v2()` with ALTER TABLE statements |
| `crates/kanban-storage/src/lib.rs` | Call `run_migrations_v2` after `run_migrations` |
| `crates/kanban-core/src/space.rs` | Extend `MemberProfile`, `Member`, `UserProfile`; update `add_member`, `list_members` |
| `crates/kanban-storage/src/space.rs` | Extend `get_profile`, `upsert_profile`, `get_space`, `upsert_member` |
| `crates/kanban-net/src/lib.rs` | Add `NetCommand::GetPeerPubkeys` + `get_peer_pubkeys_sync()` |
| `crates/kanban-net/src/swarm.rs` | Handle `GetPeerPubkeys` command |
| `crates/kanban-tauri/src-tauri/src/main.rs` | Extend view structs; update commands; add `upload_avatar_cmd`; return `peer_profiles` |
| `crates/kanban-tauri/src/index.html` | Profile modal redesign; identity chips; member list; assignee chips |

---

## Task 1: Schema Migration v2

**Files:**
- Modify: `crates/kanban-storage/src/schema.rs`
- Modify: `crates/kanban-storage/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/kanban-storage/src/schema.rs` at the end of the `#[cfg(test)]` block:

```rust
#[test]
fn v2_migration_adds_profile_and_board_columns() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn).unwrap();
    run_migrations_v2(&conn).unwrap();
    // bio column on user_profile
    conn.execute(
        "INSERT INTO user_profile (pk, pubkey, bio, role, color_accent, presence) VALUES ('local', 'pk', 'hi', 'dev', '#fff', 'online')",
        [],
    ).unwrap();
    let bio: String = conn.query_row(
        "SELECT bio FROM user_profile WHERE pk='local'", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(bio, "hi");
    // is_system column on boards
    conn.execute(
        "INSERT INTO boards (board_id, automerge_doc, is_system) VALUES ('b1', x'', 1)",
        [],
    ).unwrap();
    let sys: i64 = conn.query_row(
        "SELECT is_system FROM boards WHERE board_id='b1'", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(sys, 1);
    // idempotent
    run_migrations_v2(&conn).unwrap();
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo test -p kanban-storage v2_migration_adds_profile_and_board_columns
```
Expected: FAIL (function `run_migrations_v2` not defined)

- [ ] **Step 3: Implement `run_migrations_v2`**

Add to the end of `crates/kanban-storage/src/schema.rs`, before `#[cfg(test)]`:

```rust
pub fn run_migrations_v2(conn: &Connection) -> Result<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);
    if version >= 2 {
        return Ok(());
    }
    conn.execute_batch("
        BEGIN;
        ALTER TABLE user_profile ADD COLUMN bio TEXT;
        ALTER TABLE user_profile ADD COLUMN role TEXT;
        ALTER TABLE user_profile ADD COLUMN color_accent TEXT;
        ALTER TABLE user_profile ADD COLUMN presence TEXT DEFAULT 'online';
        ALTER TABLE space_members ADD COLUMN bio TEXT;
        ALTER TABLE space_members ADD COLUMN role TEXT;
        ALTER TABLE space_members ADD COLUMN color_accent TEXT;
        ALTER TABLE space_members ADD COLUMN presence TEXT DEFAULT 'online';
        ALTER TABLE boards ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0;
        COMMIT;
    ")?;
    conn.execute_batch("PRAGMA user_version = 2")?;
    Ok(())
}
```

- [ ] **Step 4: Call `run_migrations_v2` from Storage initialization**

In `crates/kanban-storage/src/lib.rs`, find the `Storage::new` or `open` function that calls `run_migrations`. Add a call immediately after:

```rust
crate::schema::run_migrations_v2(&conn)?;
```

- [ ] **Step 5: Run test to verify it passes**

```
cargo test -p kanban-storage v2_migration_adds_profile_and_board_columns
```
Expected: PASS

- [ ] **Step 6: Run all storage tests**

```
cargo test -p kanban-storage
```
Expected: all pass

- [ ] **Step 7: Commit**

```bash
git add crates/kanban-storage/src/schema.rs crates/kanban-storage/src/lib.rs
git commit -m "feat(storage): add schema migration v2 — profile fields + is_system on boards"
```

---

## Task 2: Extend Core Types (kanban-core)

**Files:**
- Modify: `crates/kanban-core/src/space.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)]` block in `crates/kanban-core/src/space.rs`:

```rust
#[test]
fn add_member_stores_and_retrieves_extended_fields() {
    let mut doc = create_space_doc("S", "owner").unwrap();
    let profile = MemberProfile {
        display_name: "Alice".into(),
        avatar_b64: "".into(),
        bio: "On vacation".into(),
        role: "Designer".into(),
        color_accent: "#c8962a".into(),
        presence: "away".into(),
        kicked: false,
    };
    add_member(&mut doc, "pk_alice", &profile).unwrap();
    let members = list_members(&doc).unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].bio.as_deref(), Some("On vacation"));
    assert_eq!(members[0].role.as_deref(), Some("Designer"));
    assert_eq!(members[0].color_accent.as_deref(), Some("#c8962a"));
    assert_eq!(members[0].presence.as_deref(), Some("away"));
}

#[test]
fn list_members_handles_missing_extended_fields_gracefully() {
    // Doc created without new fields (simulates old space doc from peer)
    let mut doc = create_space_doc("S", "owner").unwrap();
    let old_profile = MemberProfile {
        display_name: "Bob".into(),
        avatar_b64: "".into(),
        bio: "".into(),
        role: "".into(),
        color_accent: "".into(),
        presence: "".into(),
        kicked: false,
    };
    add_member(&mut doc, "pk_bob", &old_profile).unwrap();
    // Simulate old doc by manually NOT writing new keys — still should parse
    let members = list_members(&doc).unwrap();
    assert_eq!(members.len(), 1);
    // Empty strings become None
    assert!(members[0].bio.is_none());
    assert!(members[0].role.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p kanban-core add_member_stores_and_retrieves_extended_fields
```
Expected: FAIL (MemberProfile missing fields)

- [ ] **Step 3: Extend `MemberProfile` struct**

In `crates/kanban-core/src/space.rs`, replace the `MemberProfile` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberProfile {
    pub display_name: String,
    pub avatar_b64: String,
    pub bio: String,
    pub role: String,
    pub color_accent: String,
    pub presence: String,
    pub kicked: bool,
}
```

- [ ] **Step 4: Extend `Member` struct**

Replace the `Member` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub pubkey: String,
    pub display_name: Option<String>,
    pub avatar_blob: Option<Vec<u8>>,
    pub bio: Option<String>,
    pub role: Option<String>,
    pub color_accent: Option<String>,
    pub presence: Option<String>,
    pub kicked: bool,
}
```

- [ ] **Step 5: Extend `UserProfile` struct**

Replace the `UserProfile` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub pubkey: String,
    pub display_name: Option<String>,
    pub avatar_blob: Option<Vec<u8>>,
    pub bio: Option<String>,
    pub role: Option<String>,
    pub color_accent: Option<String>,
    pub presence: Option<String>,
    pub ssh_key_path: Option<String>,
}
```

- [ ] **Step 6: Update `add_member` to write new fields**

In the `add_member` function, add after the existing `doc.put` calls:

```rust
doc.put(&entry, "bio", profile.bio.as_str())?;
doc.put(&entry, "role", profile.role.as_str())?;
doc.put(&entry, "color_accent", profile.color_accent.as_str())?;
doc.put(&entry, "presence", profile.presence.as_str())?;
```

- [ ] **Step 7: Update `list_members` to read new fields**

In the `list_members` function, after the existing `display_name` / `avatar_b64` reads, add:

```rust
let bio = crate::get_string(doc, &entry, "bio")?.filter(|s| !s.is_empty());
let role = crate::get_string(doc, &entry, "role")?.filter(|s| !s.is_empty());
let color_accent = crate::get_string(doc, &entry, "color_accent")?.filter(|s| !s.is_empty());
let presence = crate::get_string(doc, &entry, "presence")?.filter(|s| !s.is_empty());
```

And update the `result.push(Member { ... })` call to include them:

```rust
result.push(Member { pubkey, display_name, avatar_blob, bio, role, color_accent, presence, kicked });
```

- [ ] **Step 8: Fix all compile errors caused by struct changes**

`MemberProfile` now requires new fields. Find every construction site in the codebase:

```
cargo build -p kanban-core 2>&1 | grep "missing field"
```

For each `MemberProfile { ... }` literal that doesn't include new fields, add:
```rust
bio: "".into(),
role: "".into(),
color_accent: "".into(),
presence: "".into(),
```

Similarly for `Member { ... }` literals missing the new fields, add:
```rust
bio: None,
role: None,
color_accent: None,
presence: None,
```

- [ ] **Step 9: Run tests**

```
cargo test -p kanban-core
```
Expected: all pass including new tests

- [ ] **Step 10: Commit**

```bash
git add crates/kanban-core/src/space.rs
git commit -m "feat(core): extend MemberProfile, Member, UserProfile with bio/role/color/presence"
```

---

## Task 3: Extend Storage Layer

**Files:**
- Modify: `crates/kanban-storage/src/space.rs`

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)]` in `crates/kanban-storage/src/space.rs`:

```rust
#[test]
fn profile_extended_fields_round_trip() {
    let conn = setup();
    crate::schema::run_migrations_v2(&conn).unwrap();
    let profile = UserProfile {
        pubkey: "pk".into(),
        display_name: Some("Alice".into()),
        avatar_blob: None,
        bio: Some("On vacation".into()),
        role: Some("Designer".into()),
        color_accent: Some("#c8962a".into()),
        presence: Some("away".into()),
        ssh_key_path: None,
    };
    upsert_profile(&conn, &profile).unwrap();
    let loaded = get_profile(&conn).unwrap().unwrap();
    assert_eq!(loaded.bio.as_deref(), Some("On vacation"));
    assert_eq!(loaded.role.as_deref(), Some("Designer"));
    assert_eq!(loaded.color_accent.as_deref(), Some("#c8962a"));
    assert_eq!(loaded.presence.as_deref(), Some("away"));
}

#[test]
fn upsert_member_extended_fields_round_trip() {
    let conn = setup();
    crate::schema::run_migrations_v2(&conn).unwrap();
    create_space(&conn, "s1", "S", "owner", b"bytes").unwrap();
    let member = Member {
        pubkey: "pk1".into(),
        display_name: Some("Alice".into()),
        avatar_blob: None,
        bio: Some("hello".into()),
        role: Some("Dev".into()),
        color_accent: Some("#fff".into()),
        presence: Some("online".into()),
        kicked: false,
    };
    upsert_member(&conn, "s1", &member).unwrap();
    let space = get_space(&conn, "s1").unwrap();
    let m = &space.members[0];
    assert_eq!(m.bio.as_deref(), Some("hello"));
    assert_eq!(m.role.as_deref(), Some("Dev"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p kanban-storage profile_extended_fields_round_trip
```
Expected: FAIL

- [ ] **Step 3: Update `get_profile`**

Replace the `get_profile` function:

```rust
pub fn get_profile(conn: &Connection) -> Result<Option<UserProfile>, StorageError> {
    match conn.query_row(
        "SELECT pubkey, display_name, avatar_blob, ssh_key_path,
                bio, role, color_accent, presence
         FROM user_profile WHERE pk = 'local'",
        [],
        |row| Ok(UserProfile {
            pubkey: row.get(0)?,
            display_name: row.get(1)?,
            avatar_blob: row.get(2)?,
            ssh_key_path: row.get(3)?,
            bio: row.get(4)?,
            role: row.get(5)?,
            color_accent: row.get(6)?,
            presence: row.get(7)?,
        }),
    ) {
        Ok(p) => Ok(Some(p)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StorageError::Sqlite(e)),
    }
}
```

- [ ] **Step 4: Update `upsert_profile`**

Replace the `upsert_profile` function:

```rust
pub fn upsert_profile(conn: &Connection, profile: &UserProfile) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO user_profile
         (pk, pubkey, display_name, avatar_blob, ssh_key_path, bio, role, color_accent, presence)
         VALUES ('local', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            profile.pubkey,
            profile.display_name,
            profile.avatar_blob,
            profile.ssh_key_path,
            profile.bio,
            profile.role,
            profile.color_accent,
            profile.presence,
        ],
    )?;
    Ok(())
}
```

- [ ] **Step 5: Update `upsert_member`**

Replace the `upsert_member` function:

```rust
pub fn upsert_member(
    conn: &Connection,
    space_id: &str,
    member: &Member,
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO space_members
         (space_id, pubkey, display_name, avatar_blob, kicked, bio, role, color_accent, presence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            space_id,
            member.pubkey,
            member.display_name,
            member.avatar_blob,
            member.kicked as i32,
            member.bio,
            member.role,
            member.color_accent,
            member.presence,
        ],
    )?;
    Ok(())
}
```

- [ ] **Step 6: Update `get_space` member query**

Replace the member query in `get_space` to also select new columns:

```rust
let mut stmt = conn.prepare(
    "SELECT pubkey, display_name, avatar_blob, kicked, bio, role, color_accent, presence
     FROM space_members WHERE space_id = ?1"
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
        bio: row.get::<_, Option<String>>(4)?.filter(|s| !s.is_empty()),
        role: row.get::<_, Option<String>>(5)?.filter(|s| !s.is_empty()),
        color_accent: row.get::<_, Option<String>>(6)?.filter(|s| !s.is_empty()),
        presence: row.get::<_, Option<String>>(7)?.filter(|s| !s.is_empty()),
    })
})?.collect::<Result<Vec<_>, _>>()?;
```

- [ ] **Step 7: Run all storage tests**

```
cargo test -p kanban-storage
```
Expected: all pass

- [ ] **Step 8: Build to check all dependents compile**

```
cargo build -p monotask 2>&1 | head -30
```
Fix any compile errors caused by new fields (UserProfile/Member struct literals need updating).

- [ ] **Step 9: Commit**

```bash
git add crates/kanban-storage/src/space.rs
git commit -m "feat(storage): extend get_profile, upsert_profile, upsert_member, get_space with new identity fields"
```

---

## Task 4: Add GetPeerPubkeys to Network Layer

**Files:**
- Modify: `crates/kanban-net/src/lib.rs`
- Modify: `crates/kanban-net/src/swarm.rs`

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)]` in `crates/kanban-net/src/lib.rs`:

```rust
#[test]
fn net_command_has_get_peer_pubkeys_variant() {
    // Compile-time check that the variant exists and has the right shape
    let (tx, _rx) = tokio::sync::oneshot::channel::<std::collections::HashMap<String, String>>();
    let _cmd = NetCommand::GetPeerPubkeys { reply: tx };
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo test -p kanban-net net_command_has_get_peer_pubkeys_variant
```
Expected: FAIL (variant doesn't exist)

- [ ] **Step 3: Add the variant to `NetCommand`**

In `crates/kanban-net/src/lib.rs`, add to the `NetCommand` enum:

```rust
GetPeerPubkeys { reply: tokio::sync::oneshot::Sender<std::collections::HashMap<String, String>> },
```

- [ ] **Step 4: Add `get_peer_pubkeys_sync()` to `NetworkHandle`**

In `crates/kanban-net/src/lib.rs`, add to the `impl NetworkHandle` block:

```rust
/// Return a map of connected peer IDs → ed25519 hex pubkeys.
/// Built from the swarm's Identify protocol cache.
pub fn get_peer_pubkeys_sync(&self) -> std::collections::HashMap<String, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _ = self.cmd_tx.blocking_send(NetCommand::GetPeerPubkeys { reply: tx });
    rx.blocking_recv().unwrap_or_default()
}
```

- [ ] **Step 5: Handle `GetPeerPubkeys` in swarm loop**

In `crates/kanban-net/src/swarm.rs`, find the `match cmd` block (inside the `loop { tokio::select! { Some(cmd) = cmd_rx.recv() => { match cmd {` section). Add a new arm:

```rust
NetCommand::GetPeerPubkeys { reply } => {
    let map: std::collections::HashMap<String, String> = pubkey_cache
        .iter()
        .filter_map(|(peer_id, pk)| {
            pk.clone().try_into_ed25519().ok().map(|ed_pk| {
                (peer_id.to_string(), hex::encode(ed_pk.to_bytes()))
            })
        })
        .collect();
    let _ = reply.send(map);
}
```

- [ ] **Step 6: Run all net tests**

```
cargo test -p kanban-net
```
Expected: all pass

- [ ] **Step 7: Full build check**

```
cargo build -p monotask 2>&1 | head -20
```

- [ ] **Step 8: Commit**

```bash
git add crates/kanban-net/src/lib.rs crates/kanban-net/src/swarm.rs
git commit -m "feat(net): add GetPeerPubkeys command to expose peer identity map"
```

---

## Task 5: Extend Tauri Commands

**Files:**
- Modify: `crates/kanban-tauri/src-tauri/src/main.rs`

- [ ] **Step 1: Extend view structs**

Find `UserProfileView` (around line 41) and replace it:

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct UserProfileView {
    pubkey: String,
    display_name: Option<String>,
    avatar_b64: Option<String>,
    bio: Option<String>,
    role: Option<String>,
    color_accent: Option<String>,
    presence: Option<String>,
    ssh_key_path: Option<String>,
}
```

Find `MemberView` (around line 23) and replace it:

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct MemberView {
    pubkey: String,
    display_name: Option<String>,
    avatar_b64: Option<String>,
    bio: Option<String>,
    role: Option<String>,
    color_accent: Option<String>,
    presence: Option<String>,
    kicked: bool,
}
```

Add a new struct for peer identity:

```rust
#[derive(serde::Serialize)]
struct PeerIdentityView {
    peer_id: String,
    pubkey: String,
    display_name: Option<String>,
    avatar_b64: Option<String>,
    role: Option<String>,
    color_accent: Option<String>,
    presence: Option<String>,
}
```

Extend `SyncInfo` to include peer_profiles:

```rust
#[derive(serde::Serialize)]
struct SyncInfo {
    connected_peers: Vec<String>,
    peer_profiles: Vec<PeerIdentityView>,
    boards: Vec<BoardSyncInfo>,
    local_peer_id: String,
}
```

- [ ] **Step 2: Update `get_my_profile` command**

Find the `get_my_profile` command. Update its return mapping to include new fields. Look for `UserProfileView { pubkey, display_name, avatar_b64, ssh_key_path }` and replace with:

```rust
Ok(UserProfileView {
    pubkey: profile.pubkey,
    display_name: profile.display_name,
    avatar_b64: profile.avatar_blob.as_ref().map(|b| {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(b)
    }),
    bio: profile.bio,
    role: profile.role,
    color_accent: profile.color_accent,
    presence: profile.presence,
    ssh_key_path: profile.ssh_key_path,
})
```

- [ ] **Step 3: Update `update_my_profile` command signature and body**

Replace the `update_my_profile` function signature and body. The new signature adds 4 optional string params:

```rust
#[tauri::command]
fn update_my_profile(
    display_name: String,
    avatar_b64: Option<String>,
    bio: Option<String>,
    role: Option<String>,
    color_accent: Option<String>,
    presence: Option<String>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
```

Inside the body, update the `UserProfile` construction:

```rust
let new_profile = kanban_core::space::UserProfile {
    pubkey: pubkey.clone(),
    display_name: if display_name.is_empty() { None } else { Some(display_name.clone()) },
    avatar_blob: avatar_blob.clone(),
    bio: bio.clone().filter(|s| !s.is_empty()),
    role: role.clone().filter(|s| !s.is_empty()),
    color_accent: color_accent.clone().filter(|s| !s.is_empty()),
    presence: presence.clone().filter(|s| !s.is_empty()),
    ssh_key_path: None,
};
```

Update the `MemberProfile` construction to include new fields:

```rust
let member_profile = kanban_core::space::MemberProfile {
    display_name: display_name.clone(),
    avatar_b64: avatar_b64.clone().unwrap_or_default(),
    bio: bio.clone().unwrap_or_default(),
    role: role.clone().unwrap_or_default(),
    color_accent: color_accent.clone().unwrap_or_default(),
    presence: presence.clone().unwrap_or_default(),
    kicked: false,
};
```

Update the `Member` SQL cache construction to include new fields:

```rust
let sql_member = kanban_core::space::Member {
    pubkey: pubkey.clone(),
    display_name: if display_name.is_empty() { None } else { Some(display_name.clone()) },
    avatar_blob: avatar_blob.clone(),
    bio: bio.clone().filter(|s| !s.is_empty()),
    role: role.clone().filter(|s| !s.is_empty()),
    color_accent: color_accent.clone().filter(|s| !s.is_empty()),
    presence: presence.clone().filter(|s| !s.is_empty()),
    kicked: false,
};
```

- [ ] **Step 4: Update `get_space_cmd` MemberView mapping**

Find where `MemberView` structs are constructed from `Member` values. Add the new fields:

```rust
MemberView {
    pubkey: m.pubkey,
    display_name: m.display_name,
    avatar_b64: m.avatar_blob.as_ref().map(|b| {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(b)
    }),
    bio: m.bio,
    role: m.role,
    color_accent: m.color_accent,
    presence: m.presence,
    kicked: m.kicked,
}
```

- [ ] **Step 5: Add `upload_avatar_cmd` (file picker)**

Add after the `import_ssh_key` command:

```rust
#[tauri::command]
async fn upload_avatar_cmd(
    app: tauri::AppHandle,
) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    use base64::Engine;
    let path = app.dialog()
        .file()
        .add_filter("Image", &["png", "jpg", "jpeg", "gif", "webp"])
        .blocking_pick_file();
    match path {
        Some(p) => {
            let bytes = std::fs::read(p.to_string()).map_err(|e| e.to_string())?;
            Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
        }
        None => Err("cancelled".into()),
    }
}
```

Register it in the `tauri::Builder` `.invoke_handler(tauri::generate_handler![...])` list.

- [ ] **Step 6: Extend `get_sync_info_cmd` to return peer profiles**

In `get_sync_info_cmd`, after the `connected_peers` and before `local_peer_id`, add:

```rust
// Cross-reference peer pubkeys with member profiles
let peer_pubkeys = if let Some(ref handle) = *net {
    handle.get_peer_pubkeys_sync()
} else {
    std::collections::HashMap::new()
};
drop(net);

let storage2 = state.storage.lock().map_err(|e| e.to_string())?;
let all_spaces = kanban_storage::space::list_spaces(storage2.conn())
    .map_err(|e| e.to_string())?;
let mut all_members: std::collections::HashMap<String, kanban_core::space::Member> = std::collections::HashMap::new();
for summary in &all_spaces {
    if let Ok(space) = kanban_storage::space::get_space(storage2.conn(), &summary.id) {
        for m in space.members {
            all_members.entry(m.pubkey.clone()).or_insert(m);
        }
    }
}
drop(storage2);

let peer_profiles: Vec<PeerIdentityView> = connected_peers.iter().map(|peer_id| {
    let pubkey = peer_pubkeys.get(peer_id).cloned().unwrap_or_default();
    let member = all_members.get(&pubkey);
    PeerIdentityView {
        peer_id: peer_id.clone(),
        pubkey: pubkey.clone(),
        display_name: member.and_then(|m| m.display_name.clone()),
        avatar_b64: member.and_then(|m| m.avatar_blob.as_ref().map(|b| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(b)
        })),
        role: member.and_then(|m| m.role.clone()),
        color_accent: member.and_then(|m| m.color_accent.clone()),
        presence: member.and_then(|m| m.presence.clone()),
    }
}).collect();
```

Update the `SyncInfo` return to include `peer_profiles`.

- [ ] **Step 7: Build to verify compilation**

```
cargo build -p monotask 2>&1
```
Fix any remaining compile errors.

- [ ] **Step 8: Commit**

```bash
git add crates/kanban-tauri/src-tauri/src/main.rs
git commit -m "feat(tauri): extend profile/member commands with identity fields; add upload_avatar_cmd; peer_profiles in sync info"
```

---

## Task 6: Profile Modal UI Redesign

**Files:**
- Modify: `crates/kanban-tauri/src/index.html`

- [ ] **Step 1: Add CSS for new profile elements**

In the `<style>` block, add after the existing `.modal` styles:

```css
/* ── Profile modal extended ──────────────────────────────────────────── */
.profile-avatar-wrap {
  position: relative; width: 72px; height: 72px; flex-shrink: 0;
}
.profile-avatar-img {
  width: 72px; height: 72px; border-radius: 50%; object-fit: cover;
  border: 3px solid var(--profile-accent, #c8962a);
}
.profile-avatar-initials {
  width: 72px; height: 72px; border-radius: 50%;
  display: flex; align-items: center; justify-content: center;
  font-size: 28px; font-weight: 700; color: #000;
  border: 3px solid var(--profile-accent, #c8962a);
}
.profile-presence-dot {
  position: absolute; bottom: 2px; right: 2px;
  width: 16px; height: 16px; border-radius: 50%; border: 2px solid #0d0d1c;
}
.profile-avatar-btns { display: flex; gap: 4px; margin-top: 6px; justify-content: center; }
.profile-avatar-btns button {
  background: #1a1a08; border: 1px solid #5a3a0a; color: #c8962a;
  border-radius: 3px; padding: 3px 8px; font-size: 10px; cursor: pointer;
}
.profile-avatar-btns button:hover { background: #2a2a10; }
.profile-color-swatches { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
.profile-swatch {
  width: 24px; height: 24px; border-radius: 50%; cursor: pointer;
  border: 2px solid transparent; transition: border-color .1s;
}
.profile-swatch.selected, .profile-swatch:hover { border-color: #fff; }
.profile-presence-row { display: flex; gap: 14px; }
.profile-presence-opt {
  display: flex; align-items: center; gap: 5px; cursor: pointer; font-size: 11px; color: #aaa;
}
.profile-presence-dot-sm {
  width: 10px; height: 10px; border-radius: 50%;
}
#camera-preview-wrap {
  display: none; flex-direction: column; gap: 8px; align-items: center;
}
#camera-preview { width: 200px; height: 150px; border-radius: 6px; background: #000; }
.btn-capture {
  background: #1a1a08; border: 1px solid #5a3a0a; color: #c8962a;
  border-radius: 4px; padding: 5px 14px; font-size: 12px; cursor: pointer;
}
```

- [ ] **Step 2: Replace profile modal HTML**

Find the existing `<div id="modal-profile" ...>` block and replace its inner content with:

```html
<div class="modal-header">
  <span>Profile</span>
  <button id="btn-close-profile" class="modal-close">×</button>
</div>
<div style="display:flex;gap:20px;align-items:flex-start;margin-bottom:14px;">
  <!-- Avatar column -->
  <div>
    <div class="profile-avatar-wrap" id="profile-avatar-wrap">
      <div class="profile-avatar-initials" id="profile-avatar-el" style="background:#c8962a;">?</div>
      <div class="profile-presence-dot" id="profile-presence-dot" style="background:#4a9a4a;"></div>
    </div>
    <div class="profile-avatar-btns">
      <button id="btn-avatar-file">📁 File</button>
      <button id="btn-avatar-camera">📷 Camera</button>
    </div>
    <!-- Camera preview (hidden by default) -->
    <div id="camera-preview-wrap">
      <video id="camera-preview" autoplay playsinline></video>
      <button class="btn-capture" id="btn-capture-photo">Capture</button>
      <button style="background:none;border:none;color:#666;font-size:11px;cursor:pointer;" id="btn-cancel-camera">Cancel</button>
    </div>
  </div>
  <!-- Fields column -->
  <div style="flex:1;display:flex;flex-direction:column;gap:10px;">
    <div>
      <div class="modal-label">Display Name</div>
      <input id="profile-name" class="modal-input" placeholder="Your name" />
    </div>
    <div>
      <div class="modal-label">Role / Title</div>
      <input id="profile-role" class="modal-input" placeholder="e.g. Designer, Dev" />
    </div>
  </div>
</div>
<!-- Bio -->
<div style="margin-bottom:12px;">
  <div class="modal-label">Status / Bio</div>
  <textarea id="profile-bio" class="modal-input" maxlength="120" rows="2" placeholder="What are you up to?"></textarea>
</div>
<!-- Accent color -->
<div style="margin-bottom:12px;">
  <div class="modal-label">Accent Color</div>
  <div class="profile-color-swatches" id="profile-color-swatches">
    <div class="profile-swatch" style="background:#c8962a;" data-color="#c8962a"></div>
    <div class="profile-swatch" style="background:#6a9a6a;" data-color="#6a9a6a"></div>
    <div class="profile-swatch" style="background:#6a7acc;" data-color="#6a7acc"></div>
    <div class="profile-swatch" style="background:#c86a6a;" data-color="#c86a6a"></div>
    <div class="profile-swatch" style="background:#9a6acc;" data-color="#9a6acc"></div>
    <div class="profile-swatch" style="background:#4a9a9a;" data-color="#4a9a9a"></div>
    <input type="color" id="profile-color-custom" style="width:24px;height:24px;border-radius:50%;border:none;cursor:pointer;padding:0;" title="Custom color" />
  </div>
</div>
<!-- Presence -->
<div style="margin-bottom:12px;">
  <div class="modal-label">Presence</div>
  <div class="profile-presence-row">
    <label class="profile-presence-opt"><div class="profile-presence-dot-sm" style="background:#4a9a4a;"></div>Online<input type="radio" name="presence" value="online"></label>
    <label class="profile-presence-opt"><div class="profile-presence-dot-sm" style="background:#c8962a;"></div>Away<input type="radio" name="presence" value="away"></label>
    <label class="profile-presence-opt"><div class="profile-presence-dot-sm" style="background:#555;"></div>Do Not Disturb<input type="radio" name="presence" value="dnd"></label>
  </div>
</div>
<!-- Pubkey -->
<div style="margin-bottom:10px;">
  <div class="modal-label">Public Key</div>
  <div style="background:#0a0a14;border:1px solid #1a1a2e;border-radius:4px;padding:6px 10px;display:flex;justify-content:space-between;align-items:center;">
    <span id="profile-pubkey" style="font-size:10px;color:#555;font-family:monospace;"></span>
    <button id="btn-copy-pubkey" style="background:none;border:none;color:#666;font-size:11px;cursor:pointer;">⎘ Copy</button>
  </div>
</div>
<!-- SSH key -->
<div style="margin-bottom:14px;">
  <div class="modal-label">SSH Key Path (optional)</div>
  <div style="display:flex;gap:6px;">
    <input id="profile-ssh-path" class="modal-input" style="flex:1;" placeholder="/Users/you/.ssh/id_ed25519" />
    <button id="btn-import-ssh" style="background:#1a1a08;border:1px solid #5a3a0a;color:#c8962a;border-radius:4px;padding:6px 12px;font-size:12px;cursor:pointer;">Import</button>
  </div>
</div>
<div style="display:flex;justify-content:flex-end;gap:8px;">
  <button class="btn-cancel" id="btn-cancel-profile">Cancel</button>
  <button class="btn-primary" id="btn-save-profile">Save Profile</button>
</div>
```

- [ ] **Step 3: Update profile JS — load**

Find the JS block that handles `openModal('modal-profile')` / loading profile. Replace the load logic to populate all new fields:

```javascript
async function openProfileModal() {
  const p = await invoke('get_my_profile').catch(() => null);
  if (!p) return;
  window._profileAvatarB64 = p.avatar_b64 || null;
  window._profileAccentColor = p.color_accent || '#c8962a';
  document.getElementById('profile-name').value = p.display_name || '';
  document.getElementById('profile-role').value = p.role || '';
  document.getElementById('profile-bio').value = p.bio || '';
  document.getElementById('profile-pubkey').textContent = p.pubkey.slice(0, 20) + '…';
  document.getElementById('profile-ssh-path').value = p.ssh_key_path || '';
  // Accent color
  updateAccentSwatch(p.color_accent || '#c8962a');
  // Presence radio
  const presenceVal = p.presence || 'online';
  document.querySelectorAll('input[name="presence"]').forEach(r => {
    r.checked = r.value === presenceVal;
  });
  // Avatar
  updateAvatarPreview(p.avatar_b64, p.display_name, p.color_accent);
  openModal('modal-profile');
}

function updateAvatarPreview(avatarB64, displayName, accentColor) {
  const el = document.getElementById('profile-avatar-el');
  const wrap = document.getElementById('profile-avatar-wrap');
  const color = accentColor || '#c8962a';
  wrap.style.setProperty('--profile-accent', color);
  if (avatarB64) {
    el.style.backgroundImage = `url(data:image/png;base64,${avatarB64})`;
    el.style.backgroundSize = 'cover';
    el.textContent = '';
    el.className = 'profile-avatar-img';
  } else {
    el.style.backgroundImage = '';
    el.style.background = color;
    el.textContent = (displayName || '?')[0].toUpperCase();
    el.className = 'profile-avatar-initials';
  }
}

function updateAccentSwatch(color) {
  window._profileAccentColor = color;
  document.querySelectorAll('.profile-swatch').forEach(s => {
    s.classList.toggle('selected', s.dataset.color === color);
  });
  document.getElementById('profile-avatar-wrap').style.setProperty('--profile-accent', color);
}
```

- [ ] **Step 4: Update profile JS — save**

Replace the save button handler:

```javascript
document.getElementById('btn-save-profile').onclick = async () => {
  const presence = document.querySelector('input[name="presence"]:checked')?.value || 'online';
  try {
    await invoke('update_my_profile', {
      displayName: document.getElementById('profile-name').value.trim(),
      avatarB64: window._profileAvatarB64 || null,
      bio: document.getElementById('profile-bio').value.trim() || null,
      role: document.getElementById('profile-role').value.trim() || null,
      colorAccent: window._profileAccentColor || null,
      presence,
    });
    closeModal('modal-profile');
    await loadSpacesSidebar();
  } catch (e) { alert('Error saving profile: ' + e); }
};
```

- [ ] **Step 5: Add file picker handler**

```javascript
document.getElementById('btn-avatar-file').onclick = async () => {
  try {
    const b64 = await invoke('upload_avatar_cmd');
    window._profileAvatarB64 = b64;
    updateAvatarPreview(b64, document.getElementById('profile-name').value, window._profileAccentColor);
  } catch (e) {
    if (e !== 'cancelled') alert('Error: ' + e);
  }
};
```

- [ ] **Step 6: Add camera capture handler**

```javascript
let _cameraStream = null;

document.getElementById('btn-avatar-camera').onclick = async () => {
  document.getElementById('camera-preview-wrap').style.display = 'flex';
  document.getElementById('btn-avatar-camera').style.display = 'none';
  document.getElementById('btn-avatar-file').style.display = 'none';
  _cameraStream = await navigator.mediaDevices.getUserMedia({ video: true });
  document.getElementById('camera-preview').srcObject = _cameraStream;
};

document.getElementById('btn-capture-photo').onclick = () => {
  const video = document.getElementById('camera-preview');
  const canvas = document.createElement('canvas');
  canvas.width = 256; canvas.height = 256;
  const ctx = canvas.getContext('2d');
  // Center-crop
  const s = Math.min(video.videoWidth, video.videoHeight);
  const x = (video.videoWidth - s) / 2;
  const y = (video.videoHeight - s) / 2;
  ctx.drawImage(video, x, y, s, s, 0, 0, 256, 256);
  const b64 = canvas.toDataURL('image/png').replace('data:image/png;base64,', '');
  window._profileAvatarB64 = b64;
  updateAvatarPreview(b64, document.getElementById('profile-name').value, window._profileAccentColor);
  stopCamera();
};

document.getElementById('btn-cancel-camera').onclick = stopCamera;

function stopCamera() {
  if (_cameraStream) { _cameraStream.getTracks().forEach(t => t.stop()); _cameraStream = null; }
  document.getElementById('camera-preview-wrap').style.display = 'none';
  document.getElementById('btn-avatar-camera').style.display = '';
  document.getElementById('btn-avatar-file').style.display = '';
}
```

- [ ] **Step 7: Add color swatch + custom color handlers**

```javascript
document.querySelectorAll('.profile-swatch').forEach(s => {
  s.onclick = () => updateAccentSwatch(s.dataset.color);
});
document.getElementById('profile-color-custom').oninput = (e) => {
  updateAccentSwatch(e.target.value);
};
```

- [ ] **Step 8: Build and manually verify profile modal opens correctly**

```
cargo build -p monotask
```

Launch app, open profile — verify all fields load and save.

- [ ] **Step 9: Commit**

```bash
git add crates/kanban-tauri/src/index.html
git commit -m "feat(ui): redesign profile modal — avatar upload/camera, bio, role, accent color, presence"
```

---

## Task 7: Identity Chips in UI

**Files:**
- Modify: `crates/kanban-tauri/src/index.html`

- [ ] **Step 1: Add CSS for identity chips**

In the `<style>` block, add:

```css
/* ── Identity chips (replaces raw peer ID chips) ────────────────────── */
.peer-identity-chip {
  display: inline-flex; align-items: center; gap: 8px;
  background: #0f0f1c; border: 1px solid #2a3a2a; border-radius: 5px;
  padding: 5px 10px; margin: 3px 4px 3px 0;
}
.peer-avatar-sm {
  width: 24px; height: 24px; border-radius: 50%;
  display: flex; align-items: center; justify-content: center;
  font-size: 10px; font-weight: 700; color: #000; flex-shrink: 0;
  background-size: cover; background-position: center;
  position: relative;
}
.peer-presence-sm {
  position: absolute; bottom: -1px; right: -1px;
  width: 7px; height: 7px; border-radius: 50%; border: 1px solid #0f0f1c;
}
.peer-info { display: flex; flex-direction: column; }
.peer-name { font-size: 11px; color: #ddd; font-weight: 500; }
.peer-role { font-size: 9px; color: #666; }
.peer-status-txt { font-size: 9px; }
.peer-status-txt.online { color: #4a9a4a; }
.peer-status-txt.away { color: #c8962a; }
.peer-status-txt.dnd { color: #777; }
```

- [ ] **Step 2: Update `refreshNetSync()` to render identity chips**

In the `refreshNetSync` function, replace the "Connected Peers" rendering block. The `info` object now has `peer_profiles: []` alongside `connected_peers`. Replace:

```javascript
// Connected Peers — use peer_profiles if available, fall back to raw IDs
const peersEl = document.getElementById('net-peers-container');
if (!info.connected_peers || info.connected_peers.length === 0) {
  peersEl.innerHTML = '<span style="color:#555;font-size:12px;">No connected peers</span>';
} else {
  const profileMap = {};
  (info.peer_profiles || []).forEach(p => { profileMap[p.peer_id] = p; });
  peersEl.innerHTML = info.connected_peers.map(peerId => {
    const p = profileMap[peerId];
    const color = (p && p.color_accent) || '#5a6a9a';
    const name = (p && p.display_name) || peerId.slice(0, 12) + '…';
    const role = (p && p.role) || '';
    const presence = (p && p.presence) || 'online';
    const presenceColor = presence === 'online' ? '#4a9a4a' : presence === 'away' ? '#c8962a' : '#555';
    const avatarStyle = (p && p.avatar_b64)
      ? `background-image:url(data:image/png;base64,${p.avatar_b64});background-size:cover;`
      : `background:${color};`;
    const initial = (p && p.display_name) ? p.display_name[0].toUpperCase() : '?';
    return `<span class="peer-identity-chip">
      <span class="peer-avatar-sm" style="${avatarStyle}">${(p && p.avatar_b64) ? '' : initial}<span class="peer-presence-sm" style="background:${presenceColor};"></span></span>
      <span class="peer-info">
        <span class="peer-name">${name}</span>
        ${role ? `<span class="peer-role">${role}</span>` : ''}
      </span>
      <span class="peer-status-txt ${presence}">${presence}</span>
    </span>`;
  }).join('');
}
```

- [ ] **Step 3: Update `renderMembersTab()` to show identity**

Find the `renderMembersTab` function. Update the member list item rendering to use the new fields. The `m` object now has `bio`, `role`, `color_accent`, `avatar_b64`, `presence`. Replace the current `<div class="member-avatar">` with:

```javascript
const color = m.color_accent || '#5a6a9a';
const initial = (m.display_name || m.pubkey)[0].toUpperCase();
const avatarHtml = m.avatar_b64
  ? `<div class="member-avatar" style="background-image:url(data:image/png;base64,${m.avatar_b64});background-size:cover;background-position:center;"></div>`
  : `<div class="member-avatar" style="background:${color};">${initial}</div>`;
const roleHtml = m.role ? `<span style="font-size:9px;color:#666;background:#111120;border-radius:2px;padding:1px 5px;margin-left:4px;">${m.role}</span>` : '';
const presenceDot = m.presence === 'away' ? '🟡' : m.presence === 'dnd' ? '⚫' : '🟢';
li.innerHTML = `
  ${avatarHtml}
  <div class="member-info" style="flex:1;">
    <div class="member-name">${m.display_name || '(unnamed)'}${roleHtml}</div>
    <div class="member-pubkey">${m.pubkey.slice(0, 16)}… ${presenceDot}</div>
    ${m.bio ? `<div style="font-size:10px;color:#666;font-style:italic;margin-top:2px;">${m.bio}</div>` : ''}
  </div>`;
```

- [ ] **Step 4: Update card assignee chips**

Find where assignee chips are rendered in card modal / card bodies. The current code checks for `m.avatar_b64`. Update to also use `m.color_accent` for the chip background when no photo:

```javascript
// When rendering an assignee chip:
const color = m.color_accent || '#5a6a9a';
if (m.avatar_b64) {
  av.style.cssText = `background-image:url(data:image/png;base64,${m.avatar_b64});background-size:cover;background-position:center;`;
  av.textContent = '';
} else {
  av.style.background = color;
  av.textContent = (m.display_name || m.pubkey)[0].toUpperCase();
}
av.title = m.display_name || m.pubkey;
```

- [ ] **Step 5: Build and verify**

```
cargo build -p monotask
```

Launch app with a second peer. Verify the Network & Sync panel shows names/avatars instead of raw peer IDs.

- [ ] **Step 6: Commit**

```bash
git add crates/kanban-tauri/src/index.html
git commit -m "feat(ui): replace raw peer IDs with identity chips; update member list and assignee chips"
```

---

## Final: Full Test Run & Version Bump

- [ ] **Step 1: Run all tests**

```
cargo test -p kanban-core -p kanban-storage -p kanban-net
```
Expected: all pass

- [ ] **Step 2: Full build**

```
cargo build -p monotask
```

- [ ] **Step 3: Bump version to 0.3.9**

In `crates/kanban-tauri/src-tauri/Cargo.toml` and `crates/kanban-tauri/src-tauri/tauri.conf.json`, change `0.3.8` → `0.3.9`. Also bump all other crates.

- [ ] **Step 4: Commit and tag**

```bash
git add -A
git commit -m "v0.3.9: rich identity — profile photos, bio, role, accent color, presence"
git tag v0.3.9
git push origin master --tags
```
