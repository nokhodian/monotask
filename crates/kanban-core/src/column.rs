use automerge::{AutoCommit, ObjType, ReadDoc, transaction::Transactable};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub id: String,
    pub title: String,
}

pub fn create_column(doc: &mut AutoCommit, title: &str) -> Result<String> {
    let col_id = uuid::Uuid::new_v4().to_string();
    let cols = crate::get_columns_list(doc)?;
    let idx = doc.length(&cols);
    let col_obj = doc.insert_object(&cols, idx, ObjType::Map)?;
    doc.put(&col_obj, "id", col_id.as_str())?;
    doc.put(&col_obj, "title", title)?;
    doc.put_object(&col_obj, "card_ids", ObjType::List)?;
    Ok(col_id)
}

pub fn rename_column(doc: &mut AutoCommit, col_obj: &automerge::ObjId, new_title: &str) -> Result<()> {
    doc.put(col_obj, "title", new_title)?;
    Ok(())
}

pub fn find_column_obj(doc: &AutoCommit, col_id: &str) -> Result<Option<automerge::ObjId>> {
    let cols = match doc.get(automerge::ROOT, "columns")? {
        Some((_, id)) => id,
        None => return Ok(None),
    };
    for i in 0..doc.length(&cols) {
        if let Some((_, obj)) = doc.get(&cols, i)? {
            if let Ok(Some(id)) = crate::get_string(doc, &obj, "id") {
                if id == col_id {
                    return Ok(Some(obj));
                }
            }
        }
    }
    Ok(None)
}

pub fn get_card_ids_list(doc: &AutoCommit, col_obj: &automerge::ObjId) -> Result<automerge::ObjId> {
    match doc.get(col_obj, "card_ids")? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::InvalidDocument("column missing card_ids".into())),
    }
}

pub fn append_card_to_column(doc: &mut AutoCommit, col_id: &str, card_id: &str) -> Result<()> {
    let col_obj = find_column_obj(doc, col_id)?
        .ok_or_else(|| crate::Error::NotFound(format!("column {col_id}")))?;
    let card_ids = get_card_ids_list(doc, &col_obj)?;
    let idx = doc.length(&card_ids);
    doc.insert(&card_ids, idx, card_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::AutoCommit;

    #[test]
    fn create_column_stores_title() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let id = create_column(&mut doc, "Backlog").unwrap();
        assert!(!id.is_empty());
        let obj = find_column_obj(&doc, &id).unwrap().unwrap();
        let title = crate::get_string(&doc, &obj, "title").unwrap();
        assert_eq!(title, Some("Backlog".to_string()));
    }
}
