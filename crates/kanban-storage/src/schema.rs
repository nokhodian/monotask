use rusqlite::{Connection, Result};

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch("
        PRAGMA journal_mode=WAL;
        PRAGMA foreign_keys=ON;

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
    ")?;
    Ok(())
}
