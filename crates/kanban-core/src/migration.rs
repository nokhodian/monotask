use automerge::{ReadDoc, transaction::Transactable};

/// One-time migration: assigns card numbers to all cards owned by `actor_pk` that
/// don't yet have a number field set in the Automerge document.
///
/// Returns list of `(card_id, number_string)` pairs that were assigned.
/// Call this at app startup for boards that existed before card numbers were introduced.
///
/// Each actor only migrates their own cards (identified by the `created_by` field
/// matching `hex::encode(actor_pk)`). This ensures idempotency across peers:
/// each peer runs the migration independently for their own cards, with no conflicts.
pub fn assign_numbers_for_actor(
    doc: &mut automerge::AutoCommit,
    actor_pk: &[u8],
    all_members: &[Vec<u8>],
) -> crate::Result<Vec<(String, String)>> {
    let actor_key = hex::encode(actor_pk);

    // First pass: collect card IDs + HLCs (immutable borrow)
    let owned_unnumbered: Vec<(String, String)> = {
        let cards_map = crate::get_cards_map_readonly(doc)?;
        let card_ids: Vec<String> = doc.keys(&cards_map).map(|k| k.to_string()).collect();
        let mut candidates = Vec::new();
        for card_id in card_ids {
            let card_obj = match doc.get(&cards_map, &card_id)? {
                Some((_, id)) => id,
                None => continue,
            };
            let created_by = crate::get_string(doc, &card_obj, "created_by")?;
            let number = crate::get_string(doc, &card_obj, "number")?;
            if created_by.as_deref() == Some(actor_key.as_str()) && number.is_none() {
                let hlc = crate::get_string(doc, &card_obj, "created_at")?.unwrap_or_default();
                candidates.push((card_id, hlc));
            }
        }
        candidates.sort_by(|a, b| a.1.cmp(&b.1));
        candidates
    };

    // Second pass: assign numbers (mutable borrow)
    let mut assigned = Vec::new();
    for (card_id, _) in &owned_unnumbered {
        let number = crate::card::assign_next_card_number(doc, actor_pk, all_members)?;
        let num_str = number.to_display();
        let cards_map = crate::get_cards_map(doc)?;
        let card_obj = match doc.get(&cards_map, card_id.as_str())? {
            Some((_, id)) => id,
            None => continue,
        };
        doc.put(&card_obj, "number", num_str.as_str())?;
        assigned.push((card_id.clone(), num_str));
    }

    Ok(assigned)
}

/// Helper used only in tests: creates a card in the doc WITHOUT setting the `number` field.
/// This simulates a card that existed before card numbers were introduced.
#[cfg(test)]
fn create_card_without_number(
    doc: &mut automerge::AutoCommit,
    col_id: &str,
    title: &str,
    actor_pk: &[u8],
) -> String {
    use automerge::{ObjType, transaction::Transactable};
    let card_id = uuid::Uuid::new_v4().to_string();
    let hlc = crate::clock::now();
    let cards_map = crate::get_cards_map(doc).unwrap();
    let card_obj = doc.put_object(&cards_map, &card_id, ObjType::Map).unwrap();
    doc.put(&card_obj, "id", card_id.as_str()).unwrap();
    doc.put(&card_obj, "title", title).unwrap();
    doc.put(&card_obj, "description", "").unwrap();
    doc.put(&card_obj, "created_at", hlc.as_str()).unwrap();
    doc.put(&card_obj, "created_by", hex::encode(actor_pk)).unwrap();
    doc.put(&card_obj, "deleted", false).unwrap();
    doc.put(&card_obj, "archived", false).unwrap();
    // Intentionally NOT setting "number" field to simulate pre-migration card
    crate::column::append_card_to_column(doc, col_id, &card_id).unwrap();
    card_id
}

#[cfg(test)]
fn get_card_number_from_doc(doc: &automerge::AutoCommit, card_id: &str) -> Option<String> {
    use automerge::ReadDoc;
    let cards_map = crate::get_cards_map_readonly(doc).ok()?;
    match doc.get(&cards_map, card_id).ok()? {
        Some((_, card_obj)) => {
            match doc.get(&card_obj, "number").ok()? {
                Some((automerge::Value::Scalar(s), _)) => {
                    if let automerge::ScalarValue::Str(text) = s.as_ref() {
                        Some(text.to_string())
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::AutoCommit;

    #[test]
    fn migrate_assigns_numbers_only_to_own_cards() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let actor_pk = vec![1u8; 32];
        let other_pk = vec![2u8; 32];
        let members = vec![actor_pk.clone(), other_pk.clone()];

        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();

        // Simulate two existing cards without numbers
        let own_card_id = create_card_without_number(&mut doc, &col_id, "Task A", &actor_pk);
        let other_card_id = create_card_without_number(&mut doc, &col_id, "Task B", &other_pk);

        let migrated = assign_numbers_for_actor(&mut doc, &actor_pk, &members).unwrap();

        assert_eq!(migrated.len(), 1);
        assert_eq!(migrated[0].0, own_card_id);
        // Our card now has a number
        assert!(get_card_number_from_doc(&doc, &own_card_id).is_some());
        // Other's card still has no number
        assert!(get_card_number_from_doc(&doc, &other_card_id).is_none());
    }

    #[test]
    fn migrate_skips_already_numbered_cards() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let actor_pk = vec![1u8; 32];
        let members = vec![actor_pk.clone()];

        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        // This card already has a number (created via create_card which assigns numbers)
        let numbered_card = crate::card::create_card(
            &mut doc, &col_id, "Already numbered", &actor_pk, &members
        ).unwrap();

        let migrated = assign_numbers_for_actor(&mut doc, &actor_pk, &members).unwrap();

        // Should not assign a new number to a card that already has one
        assert_eq!(migrated.len(), 0);
        // Original number unchanged
        let num = numbered_card.number.unwrap();
        let stored = get_card_number_from_doc(&doc, &numbered_card.id).unwrap();
        assert_eq!(stored, num.to_display());
    }
}
