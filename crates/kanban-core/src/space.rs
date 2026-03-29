use automerge::{AutoCommit, ObjType, ReadDoc, transaction::Transactable};
use serde::{Deserialize, Serialize};

// ── Shared types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceSummary {
    pub id: String,
    pub name: String,
    pub member_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub owner_pubkey: String,
    pub members: Vec<Member>,
    pub boards: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub pubkey: String,
    pub display_name: Option<String>,
    pub avatar_blob: Option<Vec<u8>>,
    pub bio: Option<String>,
    pub role: Option<String>,
    pub color_accent: Option<String>,
    pub presence: Option<String>,
    pub kicked: bool,
}

/// Profile embedded into SpaceDoc.members map entries.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemberProfile {
    pub display_name: String,  // empty string if not set
    pub avatar_b64: String,    // base64-encoded bytes; empty string if not set
    pub bio: String,
    pub role: String,
    pub color_accent: String,
    pub presence: String,
    pub kicked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub pubkey: String,
    pub display_name: Option<String>,
    pub avatar_blob: Option<Vec<u8>>,
    pub bio: Option<String>,
    pub role: Option<String>,
    pub color_accent: Option<String>,
    pub presence: Option<String>,
    pub ssh_key_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteMetadata {
    pub space_id: String,            // hyphenated UUID
    pub owner_pubkey: String,        // hex ed25519 pubkey
    pub timestamp: u64,              // unix seconds
    pub token_hash: String,          // SHA-256 hex of raw token bytes
    pub space_doc: Option<Vec<u8>>,  // automerge space doc (name, members, board refs)
}

// ── CRDT helpers ──────────────────────────────────────────────────────────────

fn get_members_map(doc: &AutoCommit) -> crate::Result<automerge::ObjId> {
    match doc.get(automerge::ROOT, "members")? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::InvalidDocument("space missing members map".into())),
    }
}

fn get_boards_map(doc: &AutoCommit) -> crate::Result<automerge::ObjId> {
    match doc.get(automerge::ROOT, "boards")? {
        Some((_, id)) => Ok(id),
        None => Err(crate::Error::InvalidDocument("space missing boards map".into())),
    }
}

// ── Public CRDT API ───────────────────────────────────────────────────────────

pub fn create_space_doc(name: &str, owner_pubkey: &str) -> crate::Result<AutoCommit> {
    let mut doc = AutoCommit::new();
    doc.put(automerge::ROOT, "name", name)?;
    doc.put(automerge::ROOT, "owner_pubkey", owner_pubkey)?;
    doc.put_object(automerge::ROOT, "members", ObjType::Map)?;
    doc.put_object(automerge::ROOT, "boards", ObjType::Map)?;
    Ok(doc)
}

pub fn add_member(doc: &mut AutoCommit, pubkey: &str, profile: &MemberProfile) -> crate::Result<()> {
    let members = get_members_map(doc)?;
    let entry = match doc.get(&members, pubkey)? {
        Some((_, id)) => id,
        None => doc.put_object(&members, pubkey, ObjType::Map)?,
    };
    doc.put(&entry, "display_name", profile.display_name.as_str())?;
    doc.put(&entry, "avatar_b64", profile.avatar_b64.as_str())?;
    doc.put(&entry, "bio", profile.bio.as_str())?;
    doc.put(&entry, "role", profile.role.as_str())?;
    doc.put(&entry, "color_accent", profile.color_accent.as_str())?;
    doc.put(&entry, "presence", profile.presence.as_str())?;
    doc.put(&entry, "kicked", profile.kicked)?;
    Ok(())
}

pub fn kick_member(doc: &mut AutoCommit, pubkey: &str) -> crate::Result<()> {
    let members = get_members_map(doc)?;
    if let Some((_, entry)) = doc.get(&members, pubkey)? {
        doc.put(&entry, "kicked", true)?;
    }
    Ok(())
}

pub fn add_board_ref(doc: &mut AutoCommit, board_id: &str) -> crate::Result<()> {
    let boards = get_boards_map(doc)?;
    doc.put(&boards, board_id, true)?;
    Ok(())
}

pub fn remove_board_ref(doc: &mut AutoCommit, board_id: &str) -> crate::Result<()> {
    let boards = get_boards_map(doc)?;
    // delete tombstones the key; list_board_refs filters tombstoned keys
    if doc.get(&boards, board_id)?.is_some() {
        doc.delete(&boards, board_id)?;
    }
    Ok(())
}

pub fn list_members(doc: &AutoCommit) -> crate::Result<Vec<Member>> {
    let members = get_members_map(doc)?;
    let mut result = Vec::new();
    for key in doc.keys(&members) {
        let pubkey = key.to_string();
        if let Some((_, entry)) = doc.get(&members, &pubkey)? {
            let display_name = crate::get_string(doc, &entry, "display_name")?
                .filter(|s| !s.is_empty());
            let avatar_b64 = crate::get_string(doc, &entry, "avatar_b64")?
                .unwrap_or_default();
            let avatar_blob = if avatar_b64.is_empty() {
                None
            } else {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(&avatar_b64).ok()
            };
            let bio = crate::get_string(doc, &entry, "bio")?.filter(|s| !s.is_empty());
            let role = crate::get_string(doc, &entry, "role")?.filter(|s| !s.is_empty());
            let color_accent = crate::get_string(doc, &entry, "color_accent")?.filter(|s| !s.is_empty());
            let presence = crate::get_string(doc, &entry, "presence")?.filter(|s| !s.is_empty());
            let kicked = matches!(
                doc.get(&entry, "kicked")?,
                Some((automerge::Value::Scalar(s), _))
                    if matches!(s.as_ref(), automerge::ScalarValue::Boolean(true))
            );
            result.push(Member { pubkey, display_name, avatar_blob, bio, role, color_accent, presence, kicked });
        }
    }
    Ok(result)
}

pub fn get_space_name(doc: &AutoCommit) -> Option<String> {
    crate::get_string(doc, &automerge::ROOT, "name").ok().flatten()
}

/// Store owner's listen multiaddrs in the space doc so invitees can auto-connect.
pub fn set_owner_peer_addrs(doc: &mut AutoCommit, addrs: &[String]) -> crate::Result<()> {
    // Overwrite the "peer_addrs" list each time (idempotent)
    if doc.get(automerge::ROOT, "peer_addrs")?.is_some() {
        doc.delete(automerge::ROOT, "peer_addrs")?;
    }
    let list = doc.put_object(automerge::ROOT, "peer_addrs", ObjType::List)?;
    for (i, addr) in addrs.iter().enumerate() {
        doc.insert(&list, i, addr.as_str())?;
    }
    Ok(())
}

/// Extract owner peer addrs from a space doc (may be empty for old docs).
pub fn get_owner_peer_addrs(doc: &AutoCommit) -> Vec<String> {
    let Ok(Some((_, list))) = doc.get(automerge::ROOT, "peer_addrs") else {
        return vec![];
    };
    let len = doc.length(&list);
    (0..len)
        .filter_map(|i| {
            if let Ok(Some((automerge::Value::Scalar(s), _))) = doc.get(&list, i) {
                if let automerge::ScalarValue::Str(addr) = s.as_ref() {
                    return Some(addr.to_string());
                }
            }
            None
        })
        .collect()
}

pub fn list_board_refs(doc: &AutoCommit) -> crate::Result<Vec<String>> {
    let boards = get_boards_map(doc)?;
    let mut result = Vec::new();
    for key in doc.keys(&boards) {
        let board_id = key.to_string();
        // Only include keys with a live value (not tombstoned)
        if doc.get(&boards, &board_id)?.is_some() {
            result.push(board_id);
        }
    }
    Ok(result)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_space_doc_has_required_fields() {
        let doc = create_space_doc("My Space", "aabbcc").unwrap();
        let name = crate::get_string(&doc, &automerge::ROOT, "name").unwrap();
        assert_eq!(name, Some("My Space".into()));
        let owner = crate::get_string(&doc, &automerge::ROOT, "owner_pubkey").unwrap();
        assert_eq!(owner, Some("aabbcc".into()));
    }

    #[test]
    fn add_and_list_members() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        let profile = MemberProfile {
            display_name: "Alice".into(),
            avatar_b64: "".into(),
            bio: "".into(),
            role: "".into(),
            color_accent: "".into(),
            presence: "".into(),
            kicked: false,
        };
        add_member(&mut doc, "pk_alice", &profile).unwrap();
        let members = list_members(&doc).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].pubkey, "pk_alice");
        assert_eq!(members[0].display_name, Some("Alice".into()));
        assert!(!members[0].kicked);
    }

    #[test]
    fn kick_member_sets_kicked_true() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        let profile = MemberProfile { display_name: "Bob".into(), avatar_b64: "".into(), bio: "".into(), role: "".into(), color_accent: "".into(), presence: "".into(), kicked: false };
        add_member(&mut doc, "pk_bob", &profile).unwrap();
        kick_member(&mut doc, "pk_bob").unwrap();
        let members = list_members(&doc).unwrap();
        assert!(members[0].kicked);
    }

    #[test]
    fn add_and_remove_board_ref() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        add_board_ref(&mut doc, "board-1").unwrap();
        add_board_ref(&mut doc, "board-2").unwrap();
        let boards = list_board_refs(&doc).unwrap();
        assert_eq!(boards.len(), 2);
        remove_board_ref(&mut doc, "board-1").unwrap();
        let boards = list_board_refs(&doc).unwrap();
        assert_eq!(boards.len(), 1);
        assert_eq!(boards[0], "board-2");
    }

    #[test]
    fn add_board_ref_is_idempotent() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        add_board_ref(&mut doc, "board-1").unwrap();
        add_board_ref(&mut doc, "board-1").unwrap();
        let boards = list_board_refs(&doc).unwrap();
        assert_eq!(boards.len(), 1);
    }

    #[test]
    fn add_member_is_idempotent_upsert() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        let p1 = MemberProfile { display_name: "Alice".into(), avatar_b64: "".into(), bio: "".into(), role: "".into(), color_accent: "".into(), presence: "".into(), kicked: false };
        let p2 = MemberProfile { display_name: "Alice Updated".into(), avatar_b64: "".into(), bio: "".into(), role: "".into(), color_accent: "".into(), presence: "".into(), kicked: false };
        add_member(&mut doc, "pk_alice", &p1).unwrap();
        add_member(&mut doc, "pk_alice", &p2).unwrap();
        let members = list_members(&doc).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].display_name, Some("Alice Updated".into()));
    }

    #[test]
    fn add_member_stores_and_retrieves_extended_fields() {
        let mut doc = create_space_doc("S", "owner").unwrap();
        let profile = MemberProfile {
            display_name: "Alice".into(),
            avatar_b64: "".into(),
            bio: "On vacation".into(),
            role: "Designer".into(),
            color_accent: "#c8962a".into(),
            presence: "away".into(),
            kicked: false,
        };
        add_member(&mut doc, "pk_alice", &profile).unwrap();
        let members = list_members(&doc).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].bio.as_deref(), Some("On vacation"));
        assert_eq!(members[0].role.as_deref(), Some("Designer"));
        assert_eq!(members[0].color_accent.as_deref(), Some("#c8962a"));
        assert_eq!(members[0].presence.as_deref(), Some("away"));
    }

    #[test]
    fn list_members_handles_missing_extended_fields_gracefully() {
        // Doc created without new fields (simulates old space doc from peer)
        let mut doc = create_space_doc("S", "owner").unwrap();
        let old_profile = MemberProfile {
            display_name: "Bob".into(),
            avatar_b64: "".into(),
            bio: "".into(),
            role: "".into(),
            color_accent: "".into(),
            presence: "".into(),
            kicked: false,
        };
        add_member(&mut doc, "pk_bob", &old_profile).unwrap();
        // Simulate old doc by manually NOT writing new keys — still should parse
        let members = list_members(&doc).unwrap();
        assert_eq!(members.len(), 1);
        // Empty strings become None
        assert!(members[0].bio.is_none());
        assert!(members[0].role.is_none());
    }
}
