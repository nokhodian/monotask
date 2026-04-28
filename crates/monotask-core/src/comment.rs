use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, transaction::Transactable};
use serde::{Deserialize, Serialize};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub text: String,
    pub created_at: String,
    pub deleted: bool,
    pub avatar_url: Option<String>,
}

pub fn get_comments_list(doc: &AutoCommit, card_obj: &ObjId) -> Result<ObjId> {
    match doc.get(card_obj, "comments")? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::InvalidDocument("card missing comments list".into())),
    }
}

pub fn add_comment(doc: &mut AutoCommit, card_id: &str, text: &str, author_key: &str) -> Result<Comment> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let comments = get_comments_list(doc, &card_obj)?;
    let idx = doc.length(&comments);
    let comment_id = uuid::Uuid::new_v4().to_string();
    let hlc = crate::clock::now();
    let c_obj = doc.insert_object(&comments, idx, ObjType::Map)?;
    doc.put(&c_obj, "id", comment_id.as_str())?;
    doc.put(&c_obj, "author", author_key)?;
    doc.put(&c_obj, "text", text)?;
    doc.put(&c_obj, "created_at", hlc.as_str())?;
    doc.put(&c_obj, "deleted", false)?;
    Ok(Comment { id: comment_id, author: author_key.into(), text: text.into(), created_at: hlc, deleted: false, avatar_url: None })
}

pub fn delete_comment(doc: &mut AutoCommit, card_id: &str, comment_id: &str) -> Result<()> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let comments = get_comments_list(doc, &card_obj)?;
    for i in 0..doc.length(&comments) {
        if let Some((_, c_obj)) = doc.get(&comments, i)? {
            if let Ok(Some(id)) = crate::get_string(doc, &c_obj, "id") {
                if id == comment_id {
                    doc.put(&c_obj, "deleted", true)?;
                    return Ok(());
                }
            }
        }
    }
    Err(crate::Error::NotFound(comment_id.into()))
}

pub fn edit_comment(doc: &mut AutoCommit, card_id: &str, comment_id: &str, new_text: &str) -> Result<()> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let comments = get_comments_list(doc, &card_obj)?;
    for i in 0..doc.length(&comments) {
        if let Some((_, c_obj)) = doc.get(&comments, i)? {
            if let Ok(Some(id)) = crate::get_string(doc, &c_obj, "id") {
                if id == comment_id {
                    doc.put(&c_obj, "text", new_text)?;
                    doc.put(&c_obj, "edited_at", crate::clock::now().as_str())?;
                    return Ok(());
                }
            }
        }
    }
    Err(crate::Error::NotFound(comment_id.into()))
}

pub fn set_comment_avatar_url(doc: &mut AutoCommit, card_id: &str, comment_id: &str, url: &str) -> Result<()> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let comments = get_comments_list(doc, &card_obj)?;
    for i in 0..doc.length(&comments) {
        if let Some((_, c_obj)) = doc.get(&comments, i)? {
            if let Ok(Some(id)) = crate::get_string(doc, &c_obj, "id") {
                if id == comment_id {
                    doc.put(&c_obj, "avatar_url", url)?;
                    return Ok(());
                }
            }
        }
    }
    Err(crate::Error::NotFound(comment_id.into()))
}

pub fn list_comments(doc: &AutoCommit, card_id: &str) -> Result<Vec<Comment>> {
    let card_obj = crate::card::get_card_obj(doc, card_id)?;
    let comments = get_comments_list(doc, &card_obj)?;
    let mut result = Vec::new();
    for i in 0..doc.length(&comments) {
        if let Some((_, c_obj)) = doc.get(&comments, i)? {
            let deleted = matches!(
                doc.get(&c_obj, "deleted")?,
                Some((automerge::Value::Scalar(s), _)) if matches!(s.as_ref(), automerge::ScalarValue::Boolean(true))
            );
            if !deleted {
                result.push(Comment {
                    id: crate::get_string(doc, &c_obj, "id")?.unwrap_or_default(),
                    author: crate::get_string(doc, &c_obj, "author")?.unwrap_or_default(),
                    text: crate::get_string(doc, &c_obj, "text")?.unwrap_or_default(),
                    created_at: crate::get_string(doc, &c_obj, "created_at")?.unwrap_or_default(),
                    deleted: false,
                    avatar_url: crate::get_string(doc, &c_obj, "avatar_url")?.filter(|s| !s.is_empty()),
                });
            }
        }
    }
    Ok(result)
}
