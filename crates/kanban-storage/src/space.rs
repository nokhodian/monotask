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
        "INSERT OR REPLACE INTO space_invites (token_hash, token, space_id, created_at, expires_at, revoked)
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

// ── Net helpers ──────────────────────────────────────────────────────────────

/// Returns board IDs associated with a Space.
pub fn get_space_boards(conn: &Connection, space_id: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT board_id FROM space_boards WHERE space_id = ?"
    )?;
    let ids = stmt.query_map([space_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(ids)
}

/// Returns true if the pubkey is an active (not kicked) member of the Space.
pub fn is_active_member(conn: &Connection, space_id: &str, pubkey_hex: &str) -> Result<bool, rusqlite::Error> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM space_members WHERE space_id = ? AND pubkey = ? AND kicked = 0",
        [space_id, pubkey_hex],
        |row| row.get(0),
    )?;
    Ok(count > 0)
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
