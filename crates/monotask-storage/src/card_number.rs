use rusqlite::OptionalExtension;
use monotask_core::card_number::CardNumber;

pub fn upsert_card_number(
    conn: &rusqlite::Connection,
    board_id: &str,
    card_id: &str,
    number: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO card_number_index (board_id, card_id, number)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(board_id, card_id) DO UPDATE SET number = excluded.number",
        rusqlite::params![board_id, card_id, number],
    )?;
    Ok(())
}

pub fn resolve_card_ref(
    conn: &rusqlite::Connection,
    board_id: &str,
    card_ref: &str,
) -> Result<String, crate::StorageError> {
    // Use CardNumber::from_str to determine if this is a card number reference
    if card_ref.parse::<CardNumber>().is_ok() {
        let result: Option<String> = conn
            .query_row(
                "SELECT card_id FROM card_number_index
                 WHERE board_id = ?1 AND number = ?2",
                rusqlite::params![board_id, card_ref],
                |row| row.get(0),
            )
            .optional()?;
        result.ok_or_else(|| {
            crate::StorageError::NotFound(format!("card {card_ref} not found in board {board_id}"))
        })
    } else {
        // UUID or other direct reference — pass through unchanged
        Ok(card_ref.to_string())
    }
}

/// Delete all card_number_index rows for a given board.
/// Called before re-syncing to remove stale entries.
pub fn clear_card_numbers_for_board(
    conn: &rusqlite::Connection,
    board_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM card_number_index WHERE board_id = ?1",
        rusqlite::params![board_id],
    )?;
    Ok(())
}

pub fn sync_card_number_index(
    conn: &rusqlite::Connection,
    board_id: &str,
    cards: &[(String, String)], // (card_id, number_string)
) -> rusqlite::Result<()> {
    for (card_id, number) in cards {
        upsert_card_number(conn, board_id, card_id, number)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::schema::run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn upsert_and_resolve_by_number() {
        let conn = test_db();
        upsert_card_number(&conn, "board1", "card-uuid-1", "a7f3-1").unwrap();
        let uuid = resolve_card_ref(&conn, "board1", "a7f3-1").unwrap();
        assert_eq!(uuid, "card-uuid-1");
    }

    #[test]
    fn resolve_uuid_passthrough() {
        let conn = test_db();
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let result = resolve_card_ref(&conn, "board1", uuid).unwrap();
        assert_eq!(result, uuid);
    }

    #[test]
    fn upsert_is_idempotent() {
        let conn = test_db();
        upsert_card_number(&conn, "board1", "card-uuid-1", "a7f3-1").unwrap();
        upsert_card_number(&conn, "board1", "card-uuid-1", "a7f3-1").unwrap();
        let uuid = resolve_card_ref(&conn, "board1", "a7f3-1").unwrap();
        assert_eq!(uuid, "card-uuid-1");
    }
}
