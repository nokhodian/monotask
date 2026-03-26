use rusqlite::{Connection, Result};

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch("
        PRAGMA journal_mode=WAL;
        PRAGMA foreign_keys=ON;

        BEGIN;

        CREATE TABLE IF NOT EXISTS boards (
            board_id      TEXT PRIMARY KEY,
            automerge_doc BLOB NOT NULL,
            last_modified INTEGER NOT NULL DEFAULT (unixepoch()),
            last_heads    TEXT
        );

        CREATE TABLE IF NOT EXISTS meta (
            board_id TEXT NOT NULL,
            key      TEXT NOT NULL,
            value    TEXT NOT NULL,
            PRIMARY KEY (board_id, key)
        );

        CREATE TABLE IF NOT EXISTS card_number_index (
            board_id TEXT NOT NULL,
            card_id  TEXT NOT NULL,
            number   TEXT NOT NULL,
            PRIMARY KEY (board_id, card_id)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_card_number_lookup
            ON card_number_index (board_id, number);

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

        CREATE TABLE IF NOT EXISTS undo_stack (
            board_id   TEXT    NOT NULL,
            actor_key  TEXT    NOT NULL,
            seq        INTEGER NOT NULL,
            action_tag TEXT    NOT NULL,
            inverse_op BLOB    NOT NULL,
            hlc        TEXT    NOT NULL,
            PRIMARY KEY (board_id, actor_key, seq)
        );

        CREATE TABLE IF NOT EXISTS redo_stack (
            board_id   TEXT    NOT NULL,
            actor_key  TEXT    NOT NULL,
            seq        INTEGER NOT NULL,
            action_tag TEXT    NOT NULL,
            forward_op BLOB    NOT NULL,
            hlc        TEXT    NOT NULL,
            PRIMARY KEY (board_id, actor_key, seq)
        );

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

        COMMIT;
    ")?;
    Ok(())
}

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
}
