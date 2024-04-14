pub mod board;
pub mod card_number;
pub mod schema;

use rusqlite::Connection;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("automerge error: {0}")]
    Automerge(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Extract (card_id, number_string) pairs from an Automerge doc.
fn extract_card_numbers(doc: &automerge::AutoCommit) -> Vec<(String, String)> {
    use automerge::ReadDoc;
    let cards_map = match kanban_core::get_cards_map_readonly(doc) {
        Ok(id) => id,
        Err(_) => return vec![],
    };
    doc.keys(&cards_map)
        .filter_map(|card_id| {
            let card_id = card_id.to_string();
            let card_obj = doc.get(&cards_map, &card_id).ok()?.map(|(_, id)| id)?;
            let number = kanban_core::get_string(doc, &card_obj, "number").ok()??;
            Some((card_id, number))
        })
        .collect()
}

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open(dir: &Path) -> Result<Self, StorageError> {
        std::fs::create_dir_all(dir)?;
        let db_path = dir.join("kanban.db");
        let conn = Connection::open(&db_path)?;
        schema::run_migrations(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        schema::run_migrations(&conn)?;
        Ok(Self { conn })
    }

    pub fn save_board(&mut self, board_id: &str, doc: &mut automerge::AutoCommit) -> Result<(), StorageError> {
        board::save_board(&self.conn, board_id, doc)?;
        // Keep the card_number_index in sync with the document
        let cards = extract_card_numbers(doc);
        if !cards.is_empty() {
            card_number::sync_card_number_index(&self.conn, board_id, &cards)
                .map_err(StorageError::Sqlite)?;
        }
        Ok(())
    }

    pub fn load_board(&self, board_id: &str) -> Result<automerge::AutoCommit, StorageError> {
        board::load_board(&self.conn, board_id)
    }

    pub fn list_board_ids(&self) -> Result<Vec<String>, StorageError> {
        board::list_board_ids(&self.conn)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Resolve a card reference: tries card number index first, falls back to UUID passthrough.
    pub fn resolve_card_ref(&self, board_id: &str, card_ref: &str) -> Result<String, StorageError> {
        card_number::resolve_card_ref(&self.conn, board_id, card_ref)
    }

    pub fn sync_card_number_index(&self, board_id: &str, cards: &[(String, String)]) -> Result<(), StorageError> {
        card_number::sync_card_number_index(&self.conn, board_id, cards)
            .map_err(StorageError::Sqlite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::AutoCommit;

    #[test]
    fn save_and_load_board_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let mut storage = Storage::open(tmp.path()).unwrap();
        let mut doc = AutoCommit::new();
        kanban_core::init_doc(&mut doc).unwrap();
        storage.save_board("board1", &mut doc).unwrap();
        let loaded = storage.load_board("board1").unwrap();
        assert!(automerge::ReadDoc::get(&loaded, automerge::ROOT, "columns").unwrap().is_some());
    }

    #[test]
    fn list_boards_returns_saved_boards() {
        let mut storage = Storage::open_in_memory().unwrap();
        let mut doc = AutoCommit::new();
        kanban_core::init_doc(&mut doc).unwrap();
        storage.save_board("board-a", &mut doc).unwrap();
        storage.save_board("board-b", &mut doc).unwrap();
        let ids = storage.list_board_ids().unwrap();
        assert_eq!(ids.len(), 2);
    }
}
