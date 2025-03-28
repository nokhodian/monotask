use automerge::AutoCommit;
use rusqlite::params;
use crate::StorageError;

pub fn save_board(conn: &rusqlite::Connection, board_id: &str, doc: &mut AutoCommit) -> Result<(), StorageError> {
    let tx = conn.unchecked_transaction()?;
    let bytes = doc.save();
    tx.execute(
        "INSERT INTO boards (board_id, automerge_doc, last_modified)
         VALUES (?1, ?2, unixepoch())
         ON CONFLICT(board_id) DO UPDATE SET
             automerge_doc = excluded.automerge_doc,
             last_modified = excluded.last_modified",
        params![board_id, bytes],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn load_board(conn: &rusqlite::Connection, board_id: &str) -> Result<AutoCommit, StorageError> {
    let bytes: Vec<u8> = conn.query_row(
        "SELECT automerge_doc FROM boards WHERE board_id = ?1",
        params![board_id],
        |r| r.get(0),
    ).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(board_id.into()),
        other => StorageError::Sqlite(other),
    })?;
    automerge::AutoCommit::load(&bytes)
        .map_err(|e| StorageError::Automerge(e.to_string()))
}

pub fn list_board_ids(conn: &rusqlite::Connection) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare("SELECT board_id FROM boards WHERE is_system = 0 ORDER BY last_modified DESC")?;
    let ids = stmt.query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids)
}

/// Returns (board_id, last_modified unix timestamp) for boards in active spaces only.
pub fn list_boards_with_timestamps(conn: &rusqlite::Connection) -> Result<Vec<(String, i64)>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT b.board_id, COALESCE(b.last_modified, 0)
         FROM boards b
         JOIN space_boards sb ON sb.board_id = b.board_id
         JOIN spaces s ON s.id = sb.space_id
         WHERE b.is_system = 0
         ORDER BY b.last_modified DESC"
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
