use automerge::{AutoCommit, ObjType, ReadDoc, ScalarValue, transaction::Transactable};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Card {
    pub id: String,
    pub number: Option<crate::card_number::CardNumber>,
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

pub(crate) fn assign_next_card_number(
    doc: &mut AutoCommit,
    actor_pk: &[u8],
    all_members: &[Vec<u8>],
) -> Result<crate::card_number::CardNumber> {
    use automerge::{ObjId, ObjType, ScalarValue};

    let actor_key = hex::encode(actor_pk);

    // Get or create actor_card_seq map at root
    let seq_map: ObjId = match doc.get(automerge::ROOT, "actor_card_seq")? {
        Some((automerge::Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(automerge::ROOT, "actor_card_seq", ObjType::Map)?,
    };

    let next_seq: u64 = match doc.get(&seq_map, &actor_key)? {
        Some((automerge::Value::Scalar(s), _)) => {
            if let ScalarValue::Counter(c) = s.as_ref() {
                // automerge Counter wraps i64; guard against malformed/adversarial documents
                let raw_i64 = i64::from(c);
                if raw_i64 < 0 {
                    return Err(crate::Error::InvalidDocument(
                        "actor_card_seq counter has negative value".into(),
                    ));
                }
                let current = raw_i64 as u64;
                doc.increment(&seq_map, &actor_key, 1)?;
                current + 1
            } else {
                return Err(crate::Error::InvalidDocument(
                    "actor_card_seq entry is not a counter".into(),
                ));
            }
        }
        _ => {
            doc.put(&seq_map, &actor_key, ScalarValue::counter(1))?;
            1
        }
    };

    let prefix = crate::card_number::actor_prefix(actor_pk, all_members);
    Ok(crate::card_number::CardNumber::new(prefix, next_seq))
}

pub fn create_card(
    doc: &mut AutoCommit,
    col_id: &str,
    title: &str,
    actor_pk: &[u8],
    all_members: &[Vec<u8>],
) -> Result<Card> {
    let card_id = uuid::Uuid::new_v4().to_string();
    let hlc = crate::clock::now();
    let number = assign_next_card_number(doc, actor_pk, all_members)?;
    let cards_map = crate::get_cards_map(doc)?;
    let card_obj = doc.put_object(&cards_map, &card_id, ObjType::Map)?;
    doc.put(&card_obj, "id", card_id.as_str())?;
    doc.put(&card_obj, "title", title)?;
    doc.put(&card_obj, "description", "")?;
    doc.put(&card_obj, "number", number.to_display())?;
    doc.put(&card_obj, "created_by", hex::encode(actor_pk))?;
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
        number: Some(number),
        created_at: hlc,
        ..Default::default()
    })
}

pub fn read_card(doc: &AutoCommit, card_id: &str) -> Result<Card> {
    let card_obj = get_card_obj(doc, card_id)?;
    let title = crate::get_string(doc, &card_obj, "title")?.unwrap_or_default();
    let description = crate::get_string(doc, &card_obj, "description")?.unwrap_or_default();
    let created_at = crate::get_string(doc, &card_obj, "created_at")?.unwrap_or_default();
    let created_by = crate::get_string(doc, &card_obj, "created_by")?.unwrap_or_default();
    let due_date = crate::get_string(doc, &card_obj, "due_date")?;
    let deleted = match doc.get(&card_obj, "deleted")? {
        Some((automerge::Value::Scalar(s), _)) => matches!(s.as_ref(), automerge::ScalarValue::Boolean(true)),
        _ => false,
    };
    let archived = match doc.get(&card_obj, "archived")? {
        Some((automerge::Value::Scalar(s), _)) => matches!(s.as_ref(), automerge::ScalarValue::Boolean(true)),
        _ => false,
    };
    Ok(Card {
        id: card_id.to_string(),
        title,
        description,
        created_at,
        created_by,
        due_date,
        deleted,
        archived,
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

/// Duplicate a card into the same or different column.
///
/// Copies: title (prefixed "Copy of"), description, labels.
/// Resets: assignees (empty), comments (empty), checklists (not copied in MVP).
/// Sets: new UUID, new card number, `copied_from` pointing to the source card.
pub fn copy_card(
    doc: &mut AutoCommit,
    source_card_id: &str,
    target_col_id: &str,
    actor_pk: &[u8],
    all_members: &[Vec<u8>],
) -> Result<Card> {
    // Read source card fields (immutable borrow)
    let (title, description) = {
        let cards_map = crate::get_cards_map_readonly(doc)?;
        let src_obj = match doc.get(&cards_map, source_card_id)? {
            Some((_, id)) => id,
            None => return Err(crate::Error::NotFound(source_card_id.to_string())),
        };
        let title = crate::get_string(doc, &src_obj, "title")?
            .map(|t| format!("Copy of {t}"))
            .unwrap_or_else(|| "Copy of card".to_string());
        let description = crate::get_string(doc, &src_obj, "description")?.unwrap_or_default();
        (title, description)
    };

    // Assign new card number (mutable)
    let number = assign_next_card_number(doc, actor_pk, all_members)?;
    let new_card_id = uuid::Uuid::new_v4().to_string();
    let hlc = crate::clock::now();

    // Write the new card (mutable borrow)
    let cards_map = crate::get_cards_map(doc)?;
    let card_obj = doc.put_object(&cards_map, &new_card_id, ObjType::Map)?;
    doc.put(&card_obj, "id", new_card_id.as_str())?;
    doc.put(&card_obj, "title", title.as_str())?;
    doc.put(&card_obj, "description", description.as_str())?;
    doc.put(&card_obj, "number", number.to_display())?;
    doc.put(&card_obj, "created_by", hex::encode(actor_pk))?;
    doc.put(&card_obj, "created_at", hlc.as_str())?;
    doc.put(&card_obj, "copied_from", source_card_id)?;
    doc.put(&card_obj, "deleted", false)?;
    doc.put(&card_obj, "archived", false)?;
    doc.put_object(&card_obj, "assignees", ObjType::List)?;
    doc.put_object(&card_obj, "labels", ObjType::List)?;
    doc.put_object(&card_obj, "comments", ObjType::List)?;
    doc.put_object(&card_obj, "checklists", ObjType::List)?;
    doc.put_object(&card_obj, "related", ObjType::Map)?;

    // Append to target column
    crate::column::append_card_to_column(doc, target_col_id, &new_card_id)?;

    Ok(Card {
        id: new_card_id,
        title,
        description,
        number: Some(number),
        copied_from: Some(source_card_id.to_string()),
        created_at: hlc,
        created_by: hex::encode(actor_pk),
        ..Default::default()
    })
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
        let actor_pk = vec![0u8; 32];
        let members = vec![actor_pk.clone()];
        let card = create_card(&mut doc, &col_id, "My Task", &actor_pk, &members).unwrap();
        assert_eq!(card.title, "My Task");
        assert!(!card.id.is_empty());
    }

    #[test]
    fn delete_card_sets_tombstone() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        let actor_pk = vec![0u8; 32];
        let members = vec![actor_pk.clone()];
        let card = create_card(&mut doc, &col_id, "Task", &actor_pk, &members).unwrap();
        delete_card(&mut doc, &card.id).unwrap();
        assert!(is_tombstoned(&doc, &card.id).unwrap());
    }

    #[test]
    fn create_card_assigns_number() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let actor_pk = vec![1u8; 32];
        let members = vec![actor_pk.clone()];
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        let card = create_card(&mut doc, &col_id, "My Task", &actor_pk, &members).unwrap();
        assert!(card.number.is_some());
        let num = card.number.unwrap();
        assert_eq!(num.seq, 1);
        assert!(!num.prefix.is_empty());
    }

    #[test]
    fn sequential_cards_have_increasing_seq() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let actor_pk = vec![1u8; 32];
        let members = vec![actor_pk.clone()];
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        let c1 = create_card(&mut doc, &col_id, "Task 1", &actor_pk, &members).unwrap();
        let c2 = create_card(&mut doc, &col_id, "Task 2", &actor_pk, &members).unwrap();
        assert_eq!(c1.number.unwrap().seq, 1);
        assert_eq!(c2.number.unwrap().seq, 2);
    }

    #[test]
    fn copy_card_produces_new_card_with_fresh_fields() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let actor_pk = vec![1u8; 32];
        let members = vec![actor_pk.clone()];
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        let original = create_card(&mut doc, &col_id, "Deploy API", &actor_pk, &members).unwrap();

        let copy = copy_card(&mut doc, &original.id, &col_id, &actor_pk, &members).unwrap();

        assert_ne!(copy.id, original.id);
        assert_eq!(copy.title, "Copy of Deploy API");
        assert_eq!(copy.number.as_ref().unwrap().seq, 2); // seq incremented from 1
        assert!(copy.assignees.is_empty());
        assert_eq!(copy.copied_from, Some(original.id.clone()));
    }
}
