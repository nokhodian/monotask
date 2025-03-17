use automerge::{AutoCommit, ObjType, ReadDoc, transaction::Transactable};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRef {
    pub kind: String,   // "card" | "board" | "member"
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub author: String,     // pubkey hex
    pub text: String,
    pub created_at: u64,    // unix seconds
    pub refs: Vec<ChatRef>,
}

pub fn create_chat_doc() -> crate::Result<AutoCommit> {
    let mut doc = AutoCommit::new();
    doc.put_object(automerge::ROOT, "messages", ObjType::List)?;
    Ok(doc)
}

pub fn append_message(doc: &mut AutoCommit, msg: &ChatMessage) -> crate::Result<()> {
    let (_, list_id) = doc.get(automerge::ROOT, "messages")?
        .ok_or_else(|| crate::Error::InvalidDocument("chat missing messages list".into()))?;
    let len = doc.length(&list_id);
    let entry = doc.insert_object(&list_id, len, ObjType::Map)?;
    doc.put(&entry, "id", msg.id.as_str())?;
    doc.put(&entry, "author", msg.author.as_str())?;
    doc.put(&entry, "text", msg.text.as_str())?;
    doc.put(&entry, "created_at", msg.created_at)?;
    // Refs sub-list
    let refs_list = doc.put_object(&entry, "refs", ObjType::List)?;
    for (i, r) in msg.refs.iter().enumerate() {
        let ref_entry = doc.insert_object(&refs_list, i, ObjType::Map)?;
        doc.put(&ref_entry, "kind", r.kind.as_str())?;
        doc.put(&ref_entry, "id", r.id.as_str())?;
        doc.put(&ref_entry, "label", r.label.as_str())?;
    }
    Ok(())
}

pub fn list_messages(doc: &AutoCommit, limit: usize, before_ts: Option<u64>) -> crate::Result<Vec<ChatMessage>> {
    let (_, list_id) = doc.get(automerge::ROOT, "messages")?
        .ok_or_else(|| crate::Error::InvalidDocument("chat missing messages list".into()))?;
    let len = doc.length(&list_id);
    let mut msgs: Vec<ChatMessage> = Vec::new();

    for i in 0..len {
        let (_, entry) = match doc.get(&list_id, i)? {
            Some(v) => v,
            None => continue,
        };
        let id = crate::get_string(doc, &entry, "id")?.unwrap_or_default();
        let author = crate::get_string(doc, &entry, "author")?.unwrap_or_default();
        let text = crate::get_string(doc, &entry, "text")?.unwrap_or_default();
        let created_at = match doc.get(&entry, "created_at")? {
            Some((automerge::Value::Scalar(s), _)) => {
                match s.as_ref() {
                    automerge::ScalarValue::Uint(n) => *n,
                    automerge::ScalarValue::Int(n) => *n as u64,
                    _ => 0,
                }
            }
            _ => 0,
        };

        if let Some(before) = before_ts {
            if created_at >= before { continue; }
        }

        // Refs
        let refs = if let Some((_, refs_list)) = doc.get(&entry, "refs")? {
            let rlen = doc.length(&refs_list);
            (0..rlen).filter_map(|j| {
                let (_, ref_entry) = doc.get(&refs_list, j).ok()??;
                let kind = crate::get_string(doc, &ref_entry, "kind").ok()?.unwrap_or_default();
                let ref_id = crate::get_string(doc, &ref_entry, "id").ok()?.unwrap_or_default();
                let label = crate::get_string(doc, &ref_entry, "label").ok()?.unwrap_or_default();
                Some(ChatRef { kind, id: ref_id, label })
            }).collect()
        } else { vec![] };

        msgs.push(ChatMessage { id, author, text, created_at, refs });
    }

    // Newest first, then limit
    msgs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    msgs.truncate(limit);
    Ok(msgs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_chat_doc_has_messages_list() {
        let doc = create_chat_doc().unwrap();
        let (_, list_id) = doc.get(automerge::ROOT, "messages").unwrap().unwrap();
        assert_eq!(doc.length(&list_id), 0);
    }

    #[test]
    fn append_and_list_messages() {
        let mut doc = create_chat_doc().unwrap();
        let msg = ChatMessage {
            id: "m1".into(),
            author: "pk_alice".into(),
            text: "Hello world".into(),
            created_at: 1000,
            refs: vec![],
        };
        append_message(&mut doc, &msg).unwrap();
        let msgs = list_messages(&doc, 10, None).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "Hello world");
        assert_eq!(msgs[0].author, "pk_alice");
    }

    #[test]
    fn list_messages_respects_limit() {
        let mut doc = create_chat_doc().unwrap();
        for i in 0..5u64 {
            append_message(&mut doc, &ChatMessage {
                id: format!("m{i}"),
                author: "pk".into(),
                text: format!("msg {i}"),
                created_at: i * 100,
                refs: vec![],
            }).unwrap();
        }
        let msgs = list_messages(&doc, 3, None).unwrap();
        assert_eq!(msgs.len(), 3);
        // newest first
        assert_eq!(msgs[0].text, "msg 4");
    }

    #[test]
    fn append_message_with_refs() {
        let mut doc = create_chat_doc().unwrap();
        let msg = ChatMessage {
            id: "m1".into(),
            author: "pk".into(),
            text: "Check #Fix login".into(),
            created_at: 100,
            refs: vec![ChatRef { kind: "card".into(), id: "card-uuid".into(), label: "Fix login".into() }],
        };
        append_message(&mut doc, &msg).unwrap();
        let msgs = list_messages(&doc, 10, None).unwrap();
        assert_eq!(msgs[0].refs.len(), 1);
        assert_eq!(msgs[0].refs[0].kind, "card");
    }

    #[test]
    fn two_docs_merge_without_data_loss() {
        // Simulate two peers creating messages independently then merging
        let mut doc_a = create_chat_doc().unwrap();
        let mut doc_b = AutoCommit::load(&doc_a.save()).unwrap();
        append_message(&mut doc_a, &ChatMessage { id: "a1".into(), author: "alice".into(), text: "Hi".into(), created_at: 1, refs: vec![] }).unwrap();
        append_message(&mut doc_b, &ChatMessage { id: "b1".into(), author: "bob".into(), text: "Hey".into(), created_at: 2, refs: vec![] }).unwrap();
        doc_a.merge(&mut doc_b).unwrap();
        let msgs = list_messages(&doc_a, 10, None).unwrap();
        assert_eq!(msgs.len(), 2);
    }
}
