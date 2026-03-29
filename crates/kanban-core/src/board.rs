use automerge::{AutoCommit, transaction::Transactable, ROOT};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub id: String,
    pub title: String,
    pub created_at: String,
}

pub fn create_board(title: &str, created_by: &str) -> Result<(AutoCommit, Board)> {
    let mut doc = AutoCommit::new();
    crate::init_doc(&mut doc)?;
    let id = uuid::Uuid::new_v4().to_string();
    let hlc = crate::clock::now();
    doc.put(ROOT, "id", id.as_str())?;
    doc.put(ROOT, "title", title)?;
    doc.put(ROOT, "created_at", hlc.as_str())?;
    doc.put(ROOT, "created_by", created_by)?;
    let board = Board { id, title: title.into(), created_at: hlc };
    Ok((doc, board))
}

pub fn get_board_title(doc: &AutoCommit) -> Result<String> {
    crate::get_string(doc, &ROOT, "title")?
        .ok_or_else(|| crate::Error::InvalidDocument("board missing title".into()))
}
