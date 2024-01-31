use automerge::{AutoCommit, ObjType, ReadDoc, ScalarValue, transaction::Transactable};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub description: String,
    pub assignees: Vec<String>,
    pub labels: Vec<String>,
    pub due_date: Option<String>,
    pub archived: bool,
    pub deleted: bool,
    pub copied_from: Option<String>,
    pub created_by: String,
    pub created_at: String,
}

pub fn create_card(doc: &mut AutoCommit, col_id: &str, title: &str) -> Result<Card> {
    let card_id = uuid::Uuid::new_v4().to_string();
    let hlc = crate::clock::now();
    let cards_map = crate::get_cards_map(doc)?;
    let card_obj = doc.put_object(&cards_map, &card_id, ObjType::Map)?;
    doc.put(&card_obj, "id", card_id.as_str())?;
    doc.put(&card_obj, "title", title)?;
    doc.put(&card_obj, "description", "")?;
    doc.put(&card_obj, "created_at", hlc.as_str())?;
    doc.put(&card_obj, "deleted", false)?;
    doc.put(&card_obj, "archived", false)?;
    doc.put_object(&card_obj, "assignees", ObjType::List)?;
    doc.put_object(&card_obj, "labels", ObjType::List)?;
    doc.put_object(&card_obj, "comments", ObjType::List)?;
    doc.put_object(&card_obj, "checklists", ObjType::List)?;
    doc.put_object(&card_obj, "related", ObjType::Map)?;
    crate::column::append_card_to_column(doc, col_id, &card_id)?;
    Ok(Card {
        id: card_id,
        title: title.to_string(),
        created_at: hlc,
        ..Default::default()
    })
}

pub fn get_card_obj(doc: &AutoCommit, card_id: &str) -> Result<automerge::ObjId> {
    let cards_map = crate::get_cards_map_readonly(doc)?;
    match doc.get(&cards_map, card_id)? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::NotFound(format!("card {card_id}"))),
    }
}

pub fn is_tombstoned(doc: &AutoCommit, card_id: &str) -> Result<bool> {
    let cards_map = crate::get_cards_map_readonly(doc)?;
    match doc.get(&cards_map, card_id)? {
        None => Ok(true), // absent = effectively tombstoned
        Some((_, obj)) => {
            match doc.get(&obj, "deleted")? {
                Some((automerge::Value::Scalar(s), _)) => {
                    if let ScalarValue::Boolean(b) = s.as_ref() {
                        Ok(*b)
                    } else {
                        Ok(false)
                    }
                }
                _ => Ok(false),
            }
        }
    }
}

pub fn rename_card(doc: &mut AutoCommit, card_id: &str, new_title: &str) -> Result<()> {
    let cards_map = crate::get_cards_map(doc)?;
    let card_obj = match doc.get(&cards_map, card_id)? {
        Some((_, id)) => id,
        None => return Err(crate::Error::NotFound(card_id.into())),
    };
    doc.put(&card_obj, "title", new_title)?;
    Ok(())
}

pub fn delete_card(doc: &mut AutoCommit, card_id: &str) -> Result<()> {
    let cards_map = crate::get_cards_map(doc)?;
    let card_obj = match doc.get(&cards_map, card_id)? {
        Some((_, id)) => id,
        None => return Err(crate::Error::NotFound(card_id.into())),
    };
    doc.put(&card_obj, "deleted", true)?;
    doc.put(&card_obj, "deleted_at", crate::clock::now().as_str())?;
    Ok(())
}

pub fn get_card_display_name(doc: &AutoCommit, card_id: &str) -> Result<Option<String>> {
    let cards_map = crate::get_cards_map_readonly(doc)?;
    match doc.get(&cards_map, card_id)? {
        None => Ok(None),
        Some((_, obj)) => {
            let title = crate::get_string(doc, &obj, "title")?;
            let number = crate::get_string(doc, &obj, "number")?;
            match (number, title) {
                (Some(n), Some(t)) => Ok(Some(format!("#{n} — {t}"))),
                (None, Some(t)) => Ok(Some(t)),
                _ => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::AutoCommit;

    #[test]
    fn create_card_stores_title() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        let card = create_card(&mut doc, &col_id, "My Task").unwrap();
        assert_eq!(card.title, "My Task");
        assert!(!card.id.is_empty());
    }

    #[test]
    fn delete_card_sets_tombstone() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        let card = create_card(&mut doc, &col_id, "Task").unwrap();
        delete_card(&mut doc, &card.id).unwrap();
        assert!(is_tombstoned(&doc, &card.id).unwrap());
    }
}
