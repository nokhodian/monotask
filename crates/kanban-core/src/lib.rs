pub mod board;
pub mod card_number;
pub mod card;
pub mod chat;
pub mod checklist;
pub mod clock;
pub mod column;
pub mod comment;
pub mod migration;
pub mod space;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("automerge error: {0}")]
    Automerge(#[from] automerge::AutomergeError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid document: {0}")]
    InvalidDocument(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, ROOT, transaction::Transactable};

/// Initialize a new Automerge document with the p2p-kanban root structure.
pub fn init_doc(doc: &mut AutoCommit) -> Result<()> {
    // Only initialise once
    if doc.get(ROOT, "columns").ok().flatten().is_some() {
        return Ok(());
    }
    doc.put_object(ROOT, "columns", ObjType::List)?;
    doc.put_object(ROOT, "cards", ObjType::Map)?;
    doc.put_object(ROOT, "members", ObjType::Map)?;
    doc.put_object(ROOT, "actor_card_seq", ObjType::Map)?;
    doc.put_object(ROOT, "label_definitions", ObjType::Map)?;
    Ok(())
}

/// Return the `cards` map ObjId (read-write).
pub fn get_cards_map(doc: &mut AutoCommit) -> Result<ObjId> {
    match doc.get(ROOT, "cards")? {
        Some((_, id)) => Ok(id),
        None => Err(Error::InvalidDocument("missing cards map".into())),
    }
}

/// Return the `cards` map ObjId (read-only).
pub fn get_cards_map_readonly(doc: &AutoCommit) -> Result<ObjId> {
    match doc.get(ROOT, "cards")? {
        Some((_, id)) => Ok(id),
        None => Err(Error::InvalidDocument("missing cards map".into())),
    }
}

/// Return the `columns` list ObjId.
pub fn get_columns_list(doc: &mut AutoCommit) -> Result<ObjId> {
    match doc.get(ROOT, "columns")? {
        Some((_, id)) => Ok(id),
        None => Err(Error::InvalidDocument("missing columns list".into())),
    }
}

pub fn get_string(doc: &AutoCommit, obj: &ObjId, key: &str) -> Result<Option<String>> {
    match doc.get(obj, key)? {
        Some((automerge::Value::Scalar(s), _)) => {
            if let automerge::ScalarValue::Str(text) = s.as_ref() {
                Ok(Some(text.to_string()))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}
