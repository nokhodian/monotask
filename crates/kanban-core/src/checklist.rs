use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, transaction::Transactable};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: String,
    pub text: String,
    pub checked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checklist {
    pub id: String,
    pub title: String,
    pub items: Vec<ChecklistItem>,
}

fn get_checklists_list(doc: &AutoCommit, card_obj: &ObjId) -> Result<ObjId> {
    match doc.get(card_obj, "checklists")? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::InvalidDocument("card missing checklists list".into())),
    }
}

pub fn add_checklist(doc: &mut AutoCommit, card_id: &str, title: &str) -> Result<Checklist> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let cls = get_checklists_list(doc, &card_obj)?;
    let idx = doc.length(&cls);
    let cl_id = uuid::Uuid::new_v4().to_string();
    let cl_obj = doc.insert_object(&cls, idx, ObjType::Map)?;
    doc.put(&cl_obj, "id", cl_id.as_str())?;
    doc.put(&cl_obj, "title", title)?;
    doc.put_object(&cl_obj, "items", ObjType::List)?;
    Ok(Checklist { id: cl_id, title: title.into(), items: vec![] })
}

pub fn add_checklist_item(doc: &mut AutoCommit, card_id: &str, cl_id: &str, text: &str) -> Result<ChecklistItem> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let cls = get_checklists_list(doc, &card_obj)?;
    for i in 0..doc.length(&cls) {
        if let Some((_, cl_obj)) = doc.get(&cls, i)? {
            if crate::get_string(doc, &cl_obj, "id")?.as_deref() == Some(cl_id) {
                let items = match doc.get(&cl_obj, "items")? {
                    Some((_, id)) => id,
                    None => doc.put_object(&cl_obj, "items", ObjType::List)?,
                };
                let item_id = uuid::Uuid::new_v4().to_string();
                let idx = doc.length(&items);
                let item_obj = doc.insert_object(&items, idx, ObjType::Map)?;
                doc.put(&item_obj, "id", item_id.as_str())?;
                doc.put(&item_obj, "text", text)?;
                doc.put(&item_obj, "checked", false)?;
                return Ok(ChecklistItem { id: item_id, text: text.into(), checked: false });
            }
        }
    }
    Err(crate::Error::NotFound(cl_id.into()))
}

pub fn list_checklists(doc: &AutoCommit, card_id: &str) -> Result<Vec<Checklist>> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let cls = match doc.get(&card_obj, "checklists")? {
        Some((_, id)) => id,
        None => return Ok(vec![]),
    };
    let mut result = Vec::new();
    for i in 0..doc.length(&cls) {
        if let Some((_, cl_obj)) = doc.get(&cls, i)? {
            let cl_id = crate::get_string(doc, &cl_obj, "id")?.unwrap_or_default();
            let title = crate::get_string(doc, &cl_obj, "title")?.unwrap_or_default();
            let items_obj = match doc.get(&cl_obj, "items")? {
                Some((_, id)) => id,
                None => { result.push(Checklist { id: cl_id, title, items: vec![] }); continue; }
            };
            let mut items = Vec::new();
            for j in 0..doc.length(&items_obj) {
                if let Some((_, item_obj)) = doc.get(&items_obj, j)? {
                    let item_id = crate::get_string(doc, &item_obj, "id")?.unwrap_or_default();
                    let text = crate::get_string(doc, &item_obj, "text")?.unwrap_or_default();
                    let checked = match doc.get(&item_obj, "checked")? {
                        Some((automerge::Value::Scalar(s), _)) => matches!(s.as_ref(), automerge::ScalarValue::Boolean(true)),
                        _ => false,
                    };
                    items.push(ChecklistItem { id: item_id, text, checked });
                }
            }
            result.push(Checklist { id: cl_id, title, items });
        }
    }
    Ok(result)
}

pub fn delete_checklist_item(doc: &mut AutoCommit, card_id: &str, cl_id: &str, item_id: &str) -> Result<()> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let cls = get_checklists_list(doc, &card_obj)?;
    for i in 0..doc.length(&cls) {
        if let Some((_, cl_obj)) = doc.get(&cls, i)? {
            if crate::get_string(doc, &cl_obj, "id")?.as_deref() == Some(cl_id) {
                let items = match doc.get(&cl_obj, "items")? {
                    Some((_, id)) => id,
                    None => return Err(crate::Error::NotFound(cl_id.into())),
                };
                for j in 0..doc.length(&items) {
                    if let Some((_, item_obj)) = doc.get(&items, j)? {
                        if crate::get_string(doc, &item_obj, "id")?.as_deref() == Some(item_id) {
                            doc.delete(&items, j)?;
                            return Ok(());
                        }
                    }
                }
                return Err(crate::Error::NotFound(item_id.into()));
            }
        }
    }
    Err(crate::Error::NotFound(cl_id.into()))
}

pub fn set_item_checked(doc: &mut AutoCommit, card_id: &str, cl_id: &str, item_id: &str, checked: bool) -> Result<()> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let cls = get_checklists_list(doc, &card_obj)?;
    for i in 0..doc.length(&cls) {
        if let Some((_, cl_obj)) = doc.get(&cls, i)? {
            if crate::get_string(doc, &cl_obj, "id")?.as_deref() == Some(cl_id) {
                let items = match doc.get(&cl_obj, "items")? {
                    Some((_, id)) => id,
                    None => return Err(crate::Error::NotFound(cl_id.into())),
                };
                for j in 0..doc.length(&items) {
                    if let Some((_, item_obj)) = doc.get(&items, j)? {
                        if crate::get_string(doc, &item_obj, "id")?.as_deref() == Some(item_id) {
                            doc.put(&item_obj, "checked", checked)?;
                            return Ok(());
                        }
                    }
                }
                return Err(crate::Error::NotFound(item_id.into()));
            }
        }
    }
    Err(crate::Error::NotFound(cl_id.into()))
}
