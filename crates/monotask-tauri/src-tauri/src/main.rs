// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};
use monotask_storage::Storage;
use monotask_crypto::Identity;
use tauri::{Emitter, Manager};

struct AppState {
    storage: Mutex<Storage>,
    identity: Identity,
    data_dir: std::path::PathBuf,
    net: Mutex<Option<monotask_net::NetworkHandle>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SpaceSummaryView {
    id: String,
    name: String,
    member_count: usize,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct MemberView {
    pubkey: String,
    display_name: Option<String>,
    avatar_b64: Option<String>,
    bio: Option<String>,
    role: Option<String>,
    color_accent: Option<String>,
    presence: Option<String>,
    kicked: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SpaceView {
    id: String,
    name: String,
    owner_pubkey: String,
    members: Vec<MemberView>,
    boards: Vec<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct UserProfileView {
    pubkey: String,
    display_name: Option<String>,
    avatar_b64: Option<String>,
    bio: Option<String>,
    role: Option<String>,
    color_accent: Option<String>,
    presence: Option<String>,
    ssh_key_path: Option<String>,
}

#[derive(serde::Serialize)]
struct PeerIdentityView {
    peer_id: String,
    pubkey: String,
    display_name: Option<String>,
    avatar_b64: Option<String>,
    role: Option<String>,
    color_accent: Option<String>,
    presence: Option<String>,
}

fn load_identity(
    data_dir: &std::path::Path,
    conn: &rusqlite::Connection,
) -> Result<monotask_crypto::Identity, Box<dyn std::error::Error>> {
    use monotask_crypto::Identity;
    use monotask_storage::space as space_store;

    let key_path = data_dir.join("identity.key");

    // Step 1 & 2: If profile row exists and has an SSH key path, try loading it
    if let Some(profile) = space_store::get_profile(conn)? {
        if let Some(ssh_path) = &profile.ssh_key_path {
            let p = std::path::Path::new(ssh_path);
            if p.exists() {
                if let Ok(id) = monotask_crypto::import_ssh_identity(Some(p)) {
                    return Ok(id);
                }
            }
        }
    }

    // Step 3: Fall back to identity.key (runs regardless of whether profile exists)
    if key_path.exists() {
        let bytes = std::fs::read(&key_path)?;
        if bytes.len() == 32 {
            let arr: [u8; 32] = bytes.try_into().unwrap();
            return Ok(Identity::from_secret_bytes(&arr));
        }
    }

    // Step 4: Generate new identity
    let id = Identity::generate();
    std::fs::write(&key_path, id.to_secret_bytes())?;
    let new_profile = monotask_core::space::UserProfile {
        pubkey: id.public_key_hex(),
        display_name: None,
        avatar_blob: None,
        bio: None,
        role: None,
        color_accent: None,
        presence: None,
        ssh_key_path: None,
    };
    space_store::upsert_profile(conn, &new_profile)?;
    Ok(id)
}

/// Validates that a user-supplied string field is non-empty and within reasonable length.
fn validate_text(s: &str, field: &str, max_len: usize) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err(format!("{} cannot be empty", field));
    }
    if s.len() > max_len {
        return Err(format!("{} is too long (max {} bytes)", field, max_len));
    }
    Ok(())
}

// ── Space helpers ─────────────────────────────────────────────────────────────

fn space_to_view(space: monotask_core::space::Space) -> SpaceView {
    SpaceView {
        id: space.id,
        name: space.name,
        owner_pubkey: space.owner_pubkey,
        members: space.members.into_iter().map(|m| MemberView {
            pubkey: m.pubkey,
            display_name: m.display_name,
            avatar_b64: m.avatar_blob.as_ref().map(|b| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(b)
            }),
            bio: m.bio,
            role: m.role,
            color_accent: m.color_accent,
            presence: m.presence,
            kicked: m.kicked,
        }).collect(),
        boards: space.boards,
    }
}

fn local_member_profile(state: &AppState) -> monotask_core::space::MemberProfile {
    use monotask_storage::space as sp;
    let storage = match state.storage.lock() {
        Ok(s) => s,
        Err(_) => return monotask_core::space::MemberProfile::default(),
    };
    let profile = sp::get_profile(storage.conn()).ok().flatten();
    monotask_core::space::MemberProfile {
        display_name: profile.as_ref()
            .and_then(|p| p.display_name.clone())
            .unwrap_or_default(),
        avatar_b64: profile.as_ref()
            .and_then(|p| p.avatar_blob.as_ref())
            .map(|b| { use base64::Engine; base64::engine::general_purpose::STANDARD.encode(b) })
            .unwrap_or_default(),
        bio: profile.as_ref()
            .and_then(|p| p.bio.clone())
            .unwrap_or_default(),
        role: profile.as_ref()
            .and_then(|p| p.role.clone())
            .unwrap_or_default(),
        color_accent: profile.as_ref()
            .and_then(|p| p.color_accent.clone())
            .unwrap_or_default(),
        presence: profile.as_ref()
            .and_then(|p| p.presence.clone())
            .unwrap_or_default(),
        kicked: false,
    }
}

// ── Space helpers ─────────────────────────────────────────────────────────────

/// Re-announce all spaces to the P2P network. Call after creating or joining a space.
fn announce_all_spaces(state: &AppState) {
    let space_ids = {
        let storage = match state.storage.lock() { Ok(s) => s, Err(_) => return };
        monotask_storage::space::list_spaces(storage.conn())
            .map(|v| v.into_iter().map(|s| s.id).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    if space_ids.is_empty() { return; }
    let net = match state.net.lock() { Ok(n) => n, Err(_) => return };
    if let Some(ref handle) = *net {
        handle.announce_spaces_sync(space_ids);
    }
}

fn trigger_board_sync(board_id: &str, state: &tauri::State<'_, AppState>) {
    let net = match state.net.lock() { Ok(n) => n, Err(_) => return };
    if let Some(ref handle) = *net {
        handle.trigger_sync_sync(board_id.to_string());
    }
}

/// Push current board state onto undo stack before a mutation.
/// Clears the redo stack for this board (new action invalidates redo history).
fn push_undo(conn: &rusqlite::Connection, board_id: &str, actor_key: &str, action_tag: &str, doc_bytes: &[u8]) {
    // Get next seq (max + 1)
    let seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM undo_stack WHERE board_id = ?1 AND actor_key = ?2",
        rusqlite::params![board_id, actor_key],
        |r| r.get(0),
    ).unwrap_or(1);
    // Keep max 20 undo steps per board per user
    let _ = conn.execute(
        "DELETE FROM undo_stack WHERE board_id = ?1 AND actor_key = ?2 AND seq IN (
            SELECT seq FROM undo_stack WHERE board_id = ?1 AND actor_key = ?2 ORDER BY seq ASC LIMIT MAX(0, (SELECT COUNT(*) FROM undo_stack WHERE board_id = ?1 AND actor_key = ?2) - 19)
         )",
        rusqlite::params![board_id, actor_key],
    );
    let hlc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default();
    let _ = conn.execute(
        "INSERT INTO undo_stack (board_id, actor_key, seq, action_tag, inverse_op, hlc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![board_id, actor_key, seq, action_tag, doc_bytes, hlc],
    );
    // Clear redo for this board (new action invalidates redo history)
    let _ = conn.execute(
        "DELETE FROM redo_stack WHERE board_id = ?1 AND actor_key = ?2",
        rusqlite::params![board_id, actor_key],
    );
}

fn get_or_create_chat_doc(
    storage: &monotask_storage::Storage,
    space_id: &str,
) -> Result<automerge::AutoCommit, String> {
    let chat_doc_id = format!("{space_id}-chat");
    match storage.load_board(&chat_doc_id) {
        Ok(doc) => Ok(doc),
        Err(_) => {
            // Bootstrap: create empty chat doc
            let mut doc = monotask_core::chat::create_chat_doc().map_err(|e| e.to_string())?;
            let bytes = doc.save();
            storage.save_board_bytes(&chat_doc_id, &bytes, true)
                .map_err(|e| e.to_string())?;
            // Register chat doc in space board refs
            let doc_bytes = monotask_storage::space::load_space_doc(storage.conn(), space_id)
                .map_err(|e| e.to_string())?;
            let mut space_doc = automerge::AutoCommit::load(&doc_bytes).map_err(|e| e.to_string())?;
            let _ = monotask_core::space::add_board_ref(&mut space_doc, &chat_doc_id);
            let _ = monotask_storage::space::update_space_doc(storage.conn(), space_id, &space_doc.save());
            let _ = monotask_storage::space::add_board(storage.conn(), space_id, &chat_doc_id);
            Ok(doc)
        }
    }
}

// ── Space commands ────────────────────────────────────────────────────────────

#[tauri::command]
fn create_space(name: String, state: tauri::State<AppState>) -> Result<SpaceView, String> {
    validate_text(&name, "Space name", 200)?;
    let space_id = uuid::Uuid::new_v4().to_string();
    let owner_pubkey = state.identity.public_key_hex();
    let mut doc = monotask_core::space::create_space_doc(&name, &owner_pubkey)
        .map_err(|e| e.to_string())?;
    let profile = local_member_profile(&state);
    monotask_core::space::add_member(&mut doc, &owner_pubkey, &profile)
        .map_err(|e| e.to_string())?;
    let bytes = doc.save();
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    monotask_storage::space::create_space(storage.conn(), &space_id, &name, &owner_pubkey, &bytes)
        .map_err(|e| e.to_string())?;
    // Add owner as SQL member
    let owner_member = monotask_core::space::Member {
        pubkey: owner_pubkey.clone(),
        display_name: if profile.display_name.is_empty() { None } else { Some(profile.display_name.clone()) },
        avatar_blob: if profile.avatar_b64.is_empty() { None } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(&profile.avatar_b64).ok()
        },
        bio: None,
        role: None,
        color_accent: None,
        presence: None,
        kicked: false,
    };
    monotask_storage::space::upsert_member(storage.conn(), &space_id, &owner_member)
        .map_err(|e| e.to_string())?;
    let space = monotask_storage::space::get_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    drop(storage);
    announce_all_spaces(&state);
    Ok(space_to_view(space))
}

#[derive(serde::Serialize)]
struct BoardSummary {
    id: String,
    title: String,
}

#[derive(serde::Serialize)]
struct CardView {
    id: String,
    title: String,
    number: Option<String>,
    has_description: bool,
    labels: Vec<String>,
    due_date: Option<String>,
    assignee: Option<String>,
    last_move: Option<MoveEvent>,
    checklist_total: usize,
    checklist_done: usize,
    cover_color: Option<String>,
    priority: Option<String>,
}

#[derive(serde::Serialize)]
struct CommentView {
    id: String,
    author: String,
    text: String,
    created_at: String,
}

#[derive(serde::Serialize)]
struct MoveEvent {
    from_col: String,
    to_col: String,
    timestamp: String,
}

#[derive(serde::Serialize)]
struct ChecklistItemView {
    id: String,
    text: String,
    checked: bool,
}

#[derive(serde::Serialize)]
struct ChecklistView {
    id: String,
    title: String,
    items: Vec<ChecklistItemView>,
}

#[derive(serde::Serialize)]
struct CardDetailView {
    id: String,
    title: String,
    description: String,
    number: Option<String>,
    labels: Vec<String>,
    due_date: Option<String>,
    assignee: Option<String>,
    created_at: String,
    comments: Vec<CommentView>,
    history: Vec<MoveEvent>,
    checklists: Vec<ChecklistView>,
    cover_color: Option<String>,
    priority: Option<String>,
}

#[derive(serde::Serialize)]
struct ColumnView {
    id: String,
    title: String,
    cards: Vec<CardView>,
}

#[derive(serde::Serialize)]
struct BoardDetail {
    id: String,
    title: String,
    columns: Vec<ColumnView>,
}

const DEFAULT_COLUMNS: &[&str] = &["Todo", "In Progress", "Review", "Done"];

#[tauri::command]
fn create_board_cmd(title: String, state: tauri::State<AppState>) -> Result<BoardSummary, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let pk = state.identity.public_key_hex();
    let (mut doc, board) = monotask_core::board::create_board(&title, &pk)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board.id, &mut doc)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::set_cached_title(storage.conn(), &board.id, &title)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board.id, &state);
    drop(storage);
    announce_all_spaces(&state);
    Ok(BoardSummary { id: board.id, title })
}

#[tauri::command]
fn list_boards(state: tauri::State<AppState>) -> Result<Vec<BoardSummary>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let rows = monotask_storage::board::list_boards_with_titles(storage.conn())
        .map_err(|e| e.to_string())?;
    let mut boards = Vec::with_capacity(rows.len());
    for (id, cached_title) in rows {
        let title = match cached_title {
            Some(t) => t,
            None => {
                // Board predates the cached_title column — load the doc once to backfill.
                let title = monotask_storage::board::load_board(storage.conn(), &id)
                    .ok()
                    .and_then(|doc| monotask_core::board::get_board_title(&doc).ok())
                    .unwrap_or_else(|| id.clone());
                let _ = monotask_storage::board::set_cached_title(storage.conn(), &id, &title);
                title
            }
        };
        boards.push(BoardSummary { id, title });
    }
    Ok(boards)
}

#[tauri::command]
fn get_board_detail(board_id: String, state: tauri::State<AppState>) -> Result<BoardDetail, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let title = monotask_core::board::get_board_title(&doc)
        .unwrap_or_else(|_| board_id.clone());

    // Auto-create default columns if board has none
    let existing = monotask_core::column::list_columns(&doc).unwrap_or_default();
    if existing.is_empty() {
        drop(existing);
        monotask_core::init_doc(&mut doc).map_err(|e| e.to_string())?;
        for col_title in DEFAULT_COLUMNS {
            monotask_core::column::create_column(&mut doc, col_title)
                .map_err(|e| e.to_string())?;
        }
        monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
            .map_err(|e| e.to_string())?;
        trigger_board_sync(&board_id, &state);
    }

    let columns = monotask_core::column::list_columns(&doc).map_err(|e| e.to_string())?;
    let mut col_views = Vec::with_capacity(columns.len());
    for col in &columns {
        let card_ids = get_column_card_ids(&doc, &col.id);
        let mut cards = Vec::new();
        for cid in card_ids {
            if let Ok(card) = monotask_core::card::read_card(&doc, &cid) {
                if !card.deleted && !card.archived {
                    let labels = get_card_labels(&doc, &cid);
                    let history: Vec<MoveEvent> = vec![];
                    let last_move = history.into_iter().last();
                    let assignee = get_card_str_field(&doc, &cid, "assignee");
                    let cover_color = get_card_str_field(&doc, &cid, "cover_color");
                    let priority = get_card_str_field(&doc, &cid, "priority");
                    let (checklist_total, checklist_done) = monotask_core::checklist::list_checklists(&doc, &cid)
                        .unwrap_or_default()
                        .iter()
                        .flat_map(|cl| cl.items.iter())
                        .fold((0usize, 0usize), |(tot, done), item| (tot + 1, done + if item.checked { 1 } else { 0 }));
                    cards.push(CardView {
                        id: card.id.clone(),
                        title: card.title,
                        number: card.number.map(|n| n.to_display()),
                        has_description: !card.description.is_empty(),
                        due_date: card.due_date,
                        assignee,
                        labels,
                        last_move,
                        checklist_total,
                        checklist_done,
                        cover_color,
                        priority,
                    });
                }
            }
        }
        col_views.push(ColumnView { id: col.id.clone(), title: col.title.clone(), cards });
    }
    Ok(BoardDetail { id: board_id, title, columns: col_views })
}

fn get_card_history(doc: &automerge::AutoCommit, card_id: &str) -> Vec<MoveEvent> {
    use automerge::ReadDoc;
    let card_obj = match monotask_core::card::get_card_obj(doc, card_id) {
        Ok(o) => o,
        Err(e) => { eprintln!("[get_card_history] get_card_obj failed: {e}"); return vec![]; }
    };
    let hist_obj = match doc.get(&card_obj, "history") {
        Ok(Some((_, id))) => id,
        Ok(None) => { eprintln!("[get_card_history] no history key on card {card_id}"); return vec![]; }
        Err(e) => { eprintln!("[get_card_history] error reading history: {e}"); return vec![]; }
    };
    eprintln!("[get_card_history] history list length={}", doc.length(&hist_obj));
    (0..doc.length(&hist_obj))
        .filter_map(|i| {
            doc.get(&hist_obj, i).ok().flatten().and_then(|(_, obj)| {
                let from_col = monotask_core::get_string(doc, &obj, "from_col").ok().flatten().unwrap_or_default();
                let to_col   = monotask_core::get_string(doc, &obj, "to_col").ok().flatten().unwrap_or_default();
                let timestamp = monotask_core::get_string(doc, &obj, "timestamp").ok().flatten().unwrap_or_default();
                Some(MoveEvent { from_col, to_col, timestamp })
            })
        })
        .collect()
}

#[tauri::command]
fn get_card_history_cmd(
    state: tauri::State<AppState>,
    board_id: String,
    card_id: String,
) -> Result<Vec<MoveEvent>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    Ok(get_card_history(&doc, &card_id))
}

#[tauri::command]
fn export_board_cmd(
    state: tauri::State<AppState>,
    board_id: String,
) -> Result<String, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let title = monotask_core::board::get_board_title(&doc)
        .unwrap_or_else(|_| board_id.clone());
    let columns = monotask_core::column::list_columns(&doc).map_err(|e| e.to_string())?;
    let mut col_views = Vec::with_capacity(columns.len());
    for col in &columns {
        let card_ids = get_column_card_ids(&doc, &col.id);
        let mut cards = Vec::new();
        for cid in card_ids {
            if let Ok(card) = monotask_core::card::read_card(&doc, &cid) {
                if !card.deleted && !card.archived {
                    let labels = get_card_labels(&doc, &cid);
                    let history: Vec<MoveEvent> = vec![];
                    let last_move = history.into_iter().last();
                    let assignee = get_card_str_field(&doc, &cid, "assignee");
                    let cover_color = get_card_str_field(&doc, &cid, "cover_color");
                    let priority = get_card_str_field(&doc, &cid, "priority");
                    let (checklist_total, checklist_done) = monotask_core::checklist::list_checklists(&doc, &cid)
                        .unwrap_or_default()
                        .iter()
                        .flat_map(|cl| cl.items.iter())
                        .fold((0usize, 0usize), |(tot, done), item| (tot + 1, done + if item.checked { 1 } else { 0 }));
                    cards.push(CardView {
                        id: card.id.clone(),
                        title: card.title,
                        number: card.number.map(|n| n.to_display()),
                        has_description: !card.description.is_empty(),
                        due_date: card.due_date,
                        assignee,
                        labels,
                        last_move,
                        checklist_total,
                        checklist_done,
                        cover_color,
                        priority,
                    });
                }
            }
        }
        col_views.push(ColumnView { id: col.id.clone(), title: col.title.clone(), cards });
    }
    let detail = BoardDetail { id: board_id, title, columns: col_views };
    // suppress unused mut warning — doc may be mutated by auto-column init in get_board_detail
    let _ = &mut doc;
    serde_json::to_string_pretty(&detail).map_err(|e| e.to_string())
}

fn get_card_str_field(doc: &automerge::AutoCommit, card_id: &str, field: &str) -> Option<String> {
    use automerge::ReadDoc;
    let card_obj = monotask_core::card::get_card_obj(doc, card_id).ok()?;
    let (v, _) = doc.get(&card_obj, field).ok()??;
    if let automerge::Value::Scalar(s) = v {
        if let automerge::ScalarValue::Str(t) = s.as_ref() {
            let r = t.to_string();
            return if r.is_empty() { None } else { Some(r) };
        }
    }
    None
}

fn get_card_labels(doc: &automerge::AutoCommit, card_id: &str) -> Vec<String> {
    use automerge::ReadDoc;
    let card_obj = match monotask_core::card::get_card_obj(doc, card_id) {
        Ok(o) => o,
        Err(_) => return vec![],
    };
    let labels_obj = match doc.get(&card_obj, "labels") {
        Ok(Some((_, id))) => id,
        _ => return vec![],
    };
    (0..doc.length(&labels_obj))
        .filter_map(|i| {
            doc.get(&labels_obj, i).ok().flatten().and_then(|(v, _)| {
                if let automerge::Value::Scalar(s) = v {
                    if let automerge::ScalarValue::Str(t) = s.as_ref() {
                        return Some(t.to_string());
                    }
                }
                None
            })
        })
        .collect()
}

fn get_column_card_ids(doc: &automerge::AutoCommit, col_id: &str) -> Vec<String> {
    use automerge::ReadDoc;
    let col_obj = match monotask_core::column::find_column_obj(doc, col_id) {
        Ok(Some(o)) => o,
        _ => return vec![],
    };
    let card_ids_list = match monotask_core::column::get_card_ids_list(doc, &col_obj) {
        Ok(o) => o,
        _ => return vec![],
    };
    (0..doc.length(&card_ids_list))
        .filter_map(|i| {
            doc.get(&card_ids_list, i).ok().flatten().and_then(|(v, _)| {
                if let automerge::Value::Scalar(s) = v {
                    if let automerge::ScalarValue::Str(t) = s.as_ref() {
                        return Some(t.to_string());
                    }
                }
                None
            })
        })
        .collect()
}

#[tauri::command]
fn create_column_cmd(board_id: String, title: String, state: tauri::State<AppState>) -> Result<String, String> {
    validate_text(&title, "Column title", 200)?;
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let col_id = monotask_core::column::create_column(&mut doc, &title)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(col_id)
}

#[tauri::command]
fn rename_board_cmd(
    state: tauri::State<AppState>,
    board_id: String,
    new_title: String,
) -> Result<(), String> {
    validate_text(&new_title, "Board title", 200)?;
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    monotask_core::board::set_board_title(&mut doc, &new_title)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::set_cached_title(storage.conn(), &board_id, &new_title)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn rename_column_cmd(
    state: tauri::State<AppState>,
    board_id: String,
    column_id: String,
    new_title: String,
) -> Result<(), String> {
    validate_text(&new_title, "Column title", 200)?;
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    monotask_core::column::rename_column_by_id(&mut doc, &column_id, &new_title)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn move_card_cmd(
    board_id: String,
    card_id: String,
    from_col_id: String,
    to_col_id: String,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    use automerge::{ReadDoc, transaction::Transactable};
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;

    // Remove from source column
    {
        let from_obj = monotask_core::column::find_column_obj(&doc, &from_col_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("column not found: {from_col_id}"))?;
        let card_ids = monotask_core::column::get_card_ids_list(&doc, &from_obj)
            .map_err(|e| e.to_string())?;
        let len = doc.length(&card_ids);
        let idx = (0..len).find(|&i| {
            doc.get(&card_ids, i).ok().flatten()
                .and_then(|(v, _)| if let automerge::Value::Scalar(s) = v {
                    if let automerge::ScalarValue::Str(t) = s.as_ref() { Some(t.to_string()) } else { None }
                } else { None })
                .as_deref() == Some(&card_id)
        }).ok_or_else(|| format!("card {card_id} not in column {from_col_id}"))?;
        doc.delete(&card_ids, idx).map_err(|e| e.to_string())?;
    }

    // Append to destination column
    monotask_core::column::append_card_to_column(&mut doc, &to_col_id, &card_id)
        .map_err(|e| e.to_string())?;

    // Record movement history on the card
    {
        use automerge::{ObjType, transaction::Transactable};
        let columns = monotask_core::column::list_columns(&doc).unwrap_or_default();
        eprintln!("[move_card_cmd] card={card_id} columns_found={}", columns.len());
        let from_title = columns.iter().find(|c| c.id == from_col_id).map(|c| c.title.clone()).unwrap_or_else(|| from_col_id.clone());
        let to_title   = columns.iter().find(|c| c.id == to_col_id).map(|c| c.title.clone()).unwrap_or_else(|| to_col_id.clone());
        eprintln!("[move_card_cmd] from={from_title:?} to={to_title:?}");
        let card_obj = monotask_core::card::get_card_obj(&doc, &card_id).map_err(|e| e.to_string())?;
        let hist_obj = match doc.get(&card_obj, "history").map_err(|e| e.to_string())? {
            Some((_, id)) => { eprintln!("[move_card_cmd] history list exists"); id }
            None => { eprintln!("[move_card_cmd] creating history list"); doc.put_object(&card_obj, "history", ObjType::List).map_err(|e| e.to_string())? }
        };
        let idx = doc.length(&hist_obj);
        eprintln!("[move_card_cmd] inserting history event at idx={idx}");
        let ts = monotask_core::clock::now();
        let ev = doc.insert_object(&hist_obj, idx, ObjType::Map).map_err(|e| e.to_string())?;
        doc.put(&ev, "from_col",  from_title.as_str()).map_err(|e| e.to_string())?;
        doc.put(&ev, "to_col",    to_title.as_str()).map_err(|e| e.to_string())?;
        doc.put(&ev, "timestamp", ts.as_str()).map_err(|e| e.to_string())?;
        eprintln!("[move_card_cmd] history recorded OK");
    }

    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    // Update card_search_index with new column name
    {
        let columns = monotask_core::column::list_columns(&doc).unwrap_or_default();
        let to_col_title = columns.iter()
            .find(|c| c.id == to_col_id)
            .map(|c| c.title.clone())
            .unwrap_or_default();
        let _ = storage.conn().execute(
            "UPDATE card_search_index SET column_name = ?1 WHERE card_id = ?2",
            rusqlite::params![to_col_title, card_id],
        );
    }
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn create_card_cmd(board_id: String, col_id: String, title: String, state: tauri::State<AppState>) -> Result<String, String> {
    validate_text(&title, "Card title", 500)?;
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let pk = state.identity.public_key_bytes();
    let card = monotask_core::card::create_card(&mut doc, &col_id, &title, &pk, &[pk.to_vec()])
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    // Update card search index
    let card_id = card.id.clone();
    let column_name = monotask_core::column::list_columns(&doc)
        .ok()
        .and_then(|cols| cols.into_iter().find(|c| c.id == col_id).map(|c| c.title))
        .unwrap_or_else(|| col_id.clone());
    let space_id: Option<String> = storage.conn().query_row(
        "SELECT space_id FROM space_boards WHERE board_id = ?1 LIMIT 1",
        [&board_id],
        |row| row.get(0),
    ).ok();
    if let Some(space_id) = space_id {
        let _ = storage.conn().execute(
            "INSERT OR REPLACE INTO card_search_index (card_id, board_id, space_id, title, column_name)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![card_id, board_id, space_id, title, column_name],
        );
    }
    trigger_board_sync(&board_id, &state);
    Ok(card.id)
}

#[tauri::command]
fn get_card_cmd(board_id: String, card_id: String, state: tauri::State<AppState>) -> Result<CardDetailView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let card = monotask_core::card::read_card(&doc, &card_id).map_err(|e| e.to_string())?;
    let labels = get_card_labels(&doc, &card_id);
    let assignee = get_card_str_field(&doc, &card_id, "assignee");
    let cover_color = get_card_str_field(&doc, &card_id, "cover_color");
    let priority = get_card_str_field(&doc, &card_id, "priority");
    let history = get_card_history(&doc, &card_id);
    let comments = monotask_core::comment::list_comments(&doc, &card_id)
        .unwrap_or_default()
        .into_iter()
        .map(|c| CommentView { id: c.id, author: c.author, text: c.text, created_at: c.created_at })
        .collect();
    let checklists = monotask_core::checklist::list_checklists(&doc, &card_id)
        .unwrap_or_default()
        .into_iter()
        .map(|cl| ChecklistView {
            id: cl.id,
            title: cl.title,
            items: cl.items.into_iter().map(|it| ChecklistItemView {
                id: it.id, text: it.text, checked: it.checked,
            }).collect(),
        })
        .collect();
    Ok(CardDetailView {
        id: card.id,
        title: card.title,
        description: card.description,
        number: card.number.map(|n| n.to_display()),
        labels,
        due_date: card.due_date,
        assignee,
        created_at: card.created_at,
        comments,
        history,
        checklists,
        cover_color,
        priority,
    })
}

#[tauri::command]
fn update_card_cmd(
    board_id: String,
    card_id: String,
    title: String,
    description: String,
    labels: Vec<String>,
    due_date: Option<String>,
    assignee: Option<String>,
    cover_color: Option<String>,
    priority: Option<String>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    use automerge::{ReadDoc, ObjType, transaction::Transactable};
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let pre_bytes = doc.save();
    let card_obj = monotask_core::card::get_card_obj(&doc, &card_id).map_err(|e| e.to_string())?;
    doc.put(&card_obj, "title", title.as_str()).map_err(|e| e.to_string())?;
    doc.put(&card_obj, "description", description.as_str()).map_err(|e| e.to_string())?;
    let due = due_date.as_deref().unwrap_or("");
    doc.put(&card_obj, "due_date", due).map_err(|e| e.to_string())?;
    let assignee_val = assignee.as_deref().unwrap_or("");
    doc.put(&card_obj, "assignee", assignee_val).map_err(|e| e.to_string())?;
    let cover = cover_color.as_deref().unwrap_or("");
    doc.put(&card_obj, "cover_color", cover).map_err(|e| e.to_string())?;
    let priority_val = priority.as_deref().unwrap_or("");
    doc.put(&card_obj, "priority", priority_val).map_err(|e| e.to_string())?;
    // Replace labels list
    let labels_obj = match doc.get(&card_obj, "labels").map_err(|e| e.to_string())? {
        Some((_, id)) => id,
        None => doc.put_object(&card_obj, "labels", ObjType::List).map_err(|e| e.to_string())?,
    };
    let len = doc.length(&labels_obj);
    for i in (0..len).rev() {
        doc.delete(&labels_obj, i).map_err(|e| e.to_string())?;
    }
    for (i, label) in labels.iter().enumerate() {
        doc.insert(&labels_obj, i, label.as_str()).map_err(|e| e.to_string())?;
    }
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    push_undo(storage.conn(), &board_id, &state.identity.public_key_hex(), "update_card", &pre_bytes);
    // Update card search index title
    let _ = storage.conn().execute(
        "UPDATE card_search_index SET title = ?1 WHERE card_id = ?2",
        rusqlite::params![title, card_id],
    );
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn add_comment_cmd(board_id: String, card_id: String, text: String, state: tauri::State<AppState>) -> Result<CommentView, String> {
    validate_text(&text, "Comment", 10_000)?;
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let author = state.identity.public_key_hex();
    let comment = monotask_core::comment::add_comment(&mut doc, &card_id, &text, &author)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(CommentView { id: comment.id, author: comment.author, text: comment.text, created_at: comment.created_at })
}

#[tauri::command]
fn delete_comment_cmd(board_id: String, card_id: String, comment_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    monotask_core::comment::delete_comment(&mut doc, &card_id, &comment_id)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn edit_comment_cmd(
    board_id: String,
    card_id: String,
    comment_id: String,
    text: String,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    validate_text(&text, "Comment", 10_000)?;
    use automerge::transaction::Transactable;
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let card_obj = monotask_core::card::get_card_obj(&doc, &card_id).map_err(|e| e.to_string())?;
    let comments = monotask_core::comment::get_comments_list(&doc, &card_obj).map_err(|e| e.to_string())?;
    use automerge::ReadDoc;
    let len = doc.length(&comments);
    let mut found = false;
    for i in 0..len {
        if let Ok(Some((_, c_obj))) = doc.get(&comments, i) {
            if let Ok(Some(id)) = monotask_core::get_string(&doc, &c_obj, "id") {
                if id == comment_id {
                    doc.put(&c_obj, "text", text.as_str()).map_err(|e| e.to_string())?;
                    found = true;
                    break;
                }
            }
        }
    }
    if !found {
        return Err(format!("Comment not found: {comment_id}"));
    }
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn delete_card_cmd(board_id: String, card_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let pre_bytes = doc.save();
    monotask_core::card::delete_card(&mut doc, &card_id).map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    push_undo(storage.conn(), &board_id, &state.identity.public_key_hex(), "delete_card", &pre_bytes);
    // Remove from card search index
    let _ = storage.conn().execute(
        "DELETE FROM card_search_index WHERE card_id = ?1",
        [&card_id],
    );
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn reorder_card_cmd(
    board_id: String,
    col_id: String,
    card_id: String,
    new_index: usize,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    monotask_core::column::reorder_card_in_column(&mut doc, &col_id, &card_id, new_index)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn add_checklist_cmd(board_id: String, card_id: String, title: String, state: tauri::State<AppState>) -> Result<ChecklistView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let cl = monotask_core::checklist::add_checklist(&mut doc, &card_id, &title)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(ChecklistView { id: cl.id, title: cl.title, items: vec![] })
}

#[tauri::command]
fn add_checklist_item_cmd(board_id: String, card_id: String, cl_id: String, text: String, state: tauri::State<AppState>) -> Result<ChecklistItemView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let item = monotask_core::checklist::add_checklist_item(&mut doc, &card_id, &cl_id, &text)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(ChecklistItemView { id: item.id, text: item.text, checked: item.checked })
}

#[tauri::command]
fn toggle_checklist_item_cmd(board_id: String, card_id: String, cl_id: String, item_id: String, checked: bool, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    monotask_core::checklist::set_item_checked(&mut doc, &card_id, &cl_id, &item_id, checked)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn delete_checklist_item_cmd(board_id: String, card_id: String, cl_id: String, item_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    monotask_core::checklist::delete_checklist_item(&mut doc, &card_id, &cl_id, &item_id)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn delete_column_cmd(board_id: String, col_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    use automerge::{ReadDoc, transaction::Transactable};
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let pre_bytes = doc.save();
    let cols = match doc.get(automerge::ROOT, "columns").map_err(|e| e.to_string())? {
        Some((_, id)) => id,
        None => return Err("board has no columns".into()),
    };
    let len = doc.length(&cols);
    let idx = (0..len).find(|&i| {
        doc.get(&cols, i).ok().flatten()
            .and_then(|(_, obj)| monotask_core::get_string(&doc, &obj, "id").ok().flatten())
            .map(|s| s == col_id)
            .unwrap_or(false)
    }).ok_or_else(|| format!("column not found: {col_id}"))?;
    doc.delete(&cols, idx).map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    push_undo(storage.conn(), &board_id, &state.identity.public_key_hex(), "delete_column", &pre_bytes);
    trigger_board_sync(&board_id, &state);
    Ok(())
}

#[tauri::command]
fn undo_cmd(board_id: String, state: tauri::State<AppState>) -> Result<bool, String> {
    let actor_key = state.identity.public_key_hex();
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let conn = storage.conn();

    // Get most recent undo entry
    let row: Option<(i64, String, Vec<u8>)> = conn.query_row(
        "SELECT seq, action_tag, inverse_op FROM undo_stack WHERE board_id = ?1 AND actor_key = ?2 ORDER BY seq DESC LIMIT 1",
        rusqlite::params![board_id, actor_key],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    ).ok();

    let (seq, action_tag, prev_bytes) = match row {
        None => return Ok(false), // nothing to undo
        Some(r) => r,
    };

    // Save current board state to redo stack
    let mut current_doc = monotask_storage::board::load_board(conn, &board_id).map_err(|e| e.to_string())?;
    let current_bytes = current_doc.save();
    let redo_seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM redo_stack WHERE board_id = ?1 AND actor_key = ?2",
        rusqlite::params![board_id, actor_key],
        |r| r.get(0),
    ).unwrap_or(1);
    let hlc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default();
    let _ = conn.execute(
        "INSERT INTO redo_stack (board_id, actor_key, seq, action_tag, forward_op, hlc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![board_id, actor_key, redo_seq, action_tag, &current_bytes, hlc],
    );

    // Restore previous state
    let mut prev_doc = automerge::AutoCommit::load(&prev_bytes).map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(conn, &board_id, &mut prev_doc).map_err(|e| e.to_string())?;

    // Remove the undo entry
    let _ = conn.execute(
        "DELETE FROM undo_stack WHERE board_id = ?1 AND actor_key = ?2 AND seq = ?3",
        rusqlite::params![board_id, actor_key, seq],
    );

    drop(storage);
    trigger_board_sync(&board_id, &state);
    Ok(true)
}

#[tauri::command]
fn redo_cmd(board_id: String, state: tauri::State<AppState>) -> Result<bool, String> {
    let actor_key = state.identity.public_key_hex();
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let conn = storage.conn();

    let row: Option<(i64, String, Vec<u8>)> = conn.query_row(
        "SELECT seq, action_tag, forward_op FROM redo_stack WHERE board_id = ?1 AND actor_key = ?2 ORDER BY seq DESC LIMIT 1",
        rusqlite::params![board_id, actor_key],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    ).ok();

    let (seq, _action_tag, forward_bytes) = match row {
        None => return Ok(false),
        Some(r) => r,
    };

    // Save current state back to undo stack
    let mut current_doc = monotask_storage::board::load_board(conn, &board_id).map_err(|e| e.to_string())?;
    let current_bytes = current_doc.save();
    let undo_seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM undo_stack WHERE board_id = ?1 AND actor_key = ?2",
        rusqlite::params![board_id, actor_key],
        |r| r.get(0),
    ).unwrap_or(1);
    let hlc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default();
    let _ = conn.execute(
        "INSERT INTO undo_stack (board_id, actor_key, seq, action_tag, inverse_op, hlc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![board_id, actor_key, undo_seq, "redo", &current_bytes, hlc],
    );

    let mut forward_doc = automerge::AutoCommit::load(&forward_bytes).map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(conn, &board_id, &mut forward_doc).map_err(|e| e.to_string())?;

    let _ = conn.execute(
        "DELETE FROM redo_stack WHERE board_id = ?1 AND actor_key = ?2 AND seq = ?3",
        rusqlite::params![board_id, actor_key, seq],
    );

    drop(storage);
    trigger_board_sync(&board_id, &state);
    Ok(true)
}

#[tauri::command]
fn list_spaces(state: tauri::State<AppState>) -> Result<Vec<SpaceSummaryView>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let summaries = monotask_storage::space::list_spaces(storage.conn())
        .map_err(|e| e.to_string())?;
    Ok(summaries.into_iter().map(|s| SpaceSummaryView {
        id: s.id, name: s.name, member_count: s.member_count,
    }).collect())
}

#[tauri::command]
fn get_space_cmd(space_id: String, state: tauri::State<AppState>) -> Result<SpaceView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let space = monotask_storage::space::get_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    Ok(space_to_view(space))
}

/// Embed the current swarm listen addresses into the space doc so invitees can auto-connect.
fn embed_listen_addrs_in_doc(state: &AppState, space_id: &str) -> Result<Vec<u8>, String> {
    let listen_addrs = {
        let net = state.net.lock().map_err(|e| e.to_string())?;
        net.as_ref().map(|h| h.get_listen_addrs_sync()).unwrap_or_default()
    };
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let doc_bytes = monotask_storage::space::load_space_doc(storage.conn(), space_id)
        .map_err(|e| e.to_string())?;
    let mut doc = automerge::AutoCommit::load(&doc_bytes).map_err(|e| e.to_string())?;
    // Only embed non-loopback addresses that look usable
    let addrs: Vec<String> = listen_addrs.into_iter()
        .filter(|a| !a.contains("/127.0.0.1/") && !a.contains("/::1/"))
        .collect();
    monotask_core::space::set_owner_peer_addrs(&mut doc, &addrs).map_err(|e| e.to_string())?;
    let updated = doc.save();
    monotask_storage::space::update_space_doc(storage.conn(), space_id, &updated)
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

/// Shared invite generation: builds a minimal doc (name + peer addrs only), signs the token.
fn generate_invite_inner(space_id: &str, state: &AppState) -> Result<String, String> {
    // Build a minimal doc with just the space name + peer addrs — NOT the full space doc.
    // The rest syncs over P2P once connected. This keeps the token small enough for QR.
    let listen_addrs = {
        let net = state.net.lock().map_err(|e| e.to_string())?;
        net.as_ref().map(|h| h.get_listen_addrs_sync()).unwrap_or_default()
    };
    let addrs: Vec<String> = listen_addrs.into_iter()
        .filter(|a| !a.contains("/127.0.0.1/") && !a.contains("/::1/"))
        .collect();
    let space_name = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        let space = monotask_storage::space::get_space(storage.conn(), &space_id)
            .map_err(|e| e.to_string())?;
        space.name
    };
    let mut mini_doc = monotask_core::space::create_space_doc(&space_name, &state.identity.public_key_hex())
        .map_err(|e| e.to_string())?;
    monotask_core::space::set_owner_peer_addrs(&mut mini_doc, &addrs)
        .map_err(|e| e.to_string())?;
    let mini_bytes = mini_doc.save();

    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    monotask_storage::space::revoke_all_invites(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let token = monotask_crypto::generate_invite_token(&space_id, &state.identity, Some(&mini_bytes))
        .map_err(|e| e.to_string())?;
    let meta = monotask_crypto::verify_invite_token_signature(&token)
        .map_err(|e| e.to_string())?;
    monotask_storage::space::insert_invite(storage.conn(), &meta.token_hash, &token, &space_id, None)
        .map_err(|e| e.to_string())?;
    Ok(token)
}

#[tauri::command]
fn generate_invite(space_id: String, state: tauri::State<AppState>) -> Result<String, String> {
    generate_invite_inner(&space_id, &state)
}

#[tauri::command]
fn revoke_invite(space_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    monotask_storage::space::revoke_all_invites(storage.conn(), &space_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn export_invite_file(space_id: String, path: String, state: tauri::State<AppState>) -> Result<(), String> {
    // The .space file contains the token + space name. No full doc needed — data syncs via P2P.
    let token = generate_invite_inner(&space_id, &state)?;
    let space_name = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        let space = monotask_storage::space::get_space(storage.conn(), &space_id)
            .map_err(|e| e.to_string())?;
        space.name
    };
    let payload = serde_json::json!({
        "token": token,
        "space_name": space_name,
    });
    std::fs::write(&path, serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

// SPEC DEVIATION: The spec says `get_invite_qr` returns a base64 PNG. This implementation
// returns the token string instead, and the UI renders the QR using qrcode.js (CDN).
// This avoids adding a Rust image-generation dependency for an MVP.
#[tauri::command]
fn get_invite_qr(space_id: String, state: tauri::State<AppState>) -> Result<String, String> {
    // Returns the active token string; UI renders QR via qrcode.js
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    monotask_storage::space::get_active_invite_token(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No active invite token".into())
}

#[tauri::command]
fn import_invite(token_or_path: String, state: tauri::State<AppState>) -> Result<SpaceView, String> {
    let local_pubkey = state.identity.public_key_hex();

    // 1. Parse token string
    let (token, space_name_hint, space_doc_bytes) = if token_or_path.ends_with(".space")
        || std::path::Path::new(&token_or_path).exists()
    {
        let content = std::fs::read_to_string(&token_or_path).map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        let tok = v["token"].as_str().unwrap_or("").to_string();
        let name = v["space_name"].as_str().unwrap_or("Shared Space").to_string();
        let doc_b64 = v["space_doc"].as_str().unwrap_or("");
        let doc_bytes = if doc_b64.is_empty() {
            None
        } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(doc_b64).ok()
        };
        (tok, name, doc_bytes)
    } else {
        (token_or_path.clone(), "Shared Space".to_string(), None)
    };

    // 2. Verify signature
    let meta = monotask_crypto::verify_invite_token_signature(&token)
        .map_err(|e| e.to_string())?;

    // For plain token strings the space_doc_bytes was set to None above; fall back to
    // the space doc embedded inside the signed token payload itself.
    let space_doc_bytes = space_doc_bytes.or_else(|| meta.space_doc.clone());

    // 3. Check policy (owner-side only)
    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        monotask_storage::space::check_invite_policy(storage.conn(), &meta, &local_pubkey)
            .map_err(|e| e.to_string())?;

        // 4. Idempotency check — already a member, but still sync boards from the token
        // in case the previous import happened before board refs were embedded in the token.
        let already = monotask_storage::space::get_space(storage.conn(), &meta.space_id);
        if let Ok(existing) = already {
            if existing.members.iter().any(|m| m.pubkey == local_pubkey) {
                if let Some(ref doc_bytes) = space_doc_bytes {
                    if let Ok(doc) = automerge::AutoCommit::load(doc_bytes) {
                        if let Ok(board_refs) = monotask_core::space::list_board_refs(&doc) {
                            for board_id in &board_refs {
                                let _ = monotask_storage::space::add_board(
                                    storage.conn(), &meta.space_id, board_id,
                                );
                            }
                        }
                        let _ = monotask_storage::space::update_space_doc(
                            storage.conn(), &meta.space_id, doc_bytes,
                        );
                        // Auto-dial owner peer addrs embedded in the token
                        if let Ok(net) = state.net.lock() {
                            if let Some(handle) = net.as_ref() {
                                for addr in monotask_core::space::get_owner_peer_addrs(&doc) {
                                    save_peer_addr(&state.data_dir, &addr);
                                    handle.add_peer_sync(addr);
                                }
                            }
                        }
                    }
                }
                let space = monotask_storage::space::get_space(storage.conn(), &meta.space_id)
                    .map_err(|e| e.to_string())?;
                drop(storage);
                announce_all_spaces(&state);
                return Ok(space_to_view(space));
            }
        }
    }

    // Extract owner peer addrs before consuming space_doc_bytes
    let owner_peer_addrs: Vec<String> = space_doc_bytes.as_ref()
        .and_then(|b| automerge::AutoCommit::load(b).ok())
        .map(|d| monotask_core::space::get_owner_peer_addrs(&d))
        .unwrap_or_default();

    // 5–8. Create or merge space
    let (mut doc, members_to_insert, boards_to_insert, space_name) = if let Some(bytes) = space_doc_bytes {
        let doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
        let members = monotask_core::space::list_members(&doc).map_err(|e| e.to_string())?;
        let boards = monotask_core::space::list_board_refs(&doc).map_err(|e| e.to_string())?;
        // Use the name embedded in the space doc; fall back to the hint from the file
        let name = monotask_core::space::get_space_name(&doc)
            .unwrap_or(space_name_hint);
        (doc, members, boards, name)
    } else {
        let mut doc = monotask_core::space::create_space_doc(&space_name_hint, &meta.owner_pubkey)
            .map_err(|e| e.to_string())?;
        let empty = monotask_core::space::MemberProfile {
            display_name: String::new(), avatar_b64: String::new(), bio: String::new(), role: String::new(), color_accent: String::new(), presence: String::new(), kicked: false,
        };
        monotask_core::space::add_member(&mut doc, &meta.owner_pubkey, &empty)
            .map_err(|e| e.to_string())?;
        // Include stub owner so SQL space_members row is created for them
        let stub_owner = monotask_core::space::Member {
            pubkey: meta.owner_pubkey.clone(),
            display_name: None,
            avatar_blob: None,
            bio: None,
            role: None,
            color_accent: None,
            presence: None,
            kicked: false,
        };
        (doc, vec![stub_owner], vec![], space_name_hint)
    };

    // Add local user to SpaceDoc
    let local_profile = local_member_profile(&state);
    monotask_core::space::add_member(&mut doc, &local_pubkey, &local_profile)
        .map_err(|e| e.to_string())?;
    let doc_bytes = doc.save();

    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Create space row (or skip if already exists from idempotency path)
    let _ = monotask_storage::space::create_space(
        storage.conn(), &meta.space_id, &space_name, &meta.owner_pubkey, &doc_bytes,
    );
    // Insert members from snapshot
    for m in &members_to_insert {
        let _ = monotask_storage::space::upsert_member(storage.conn(), &meta.space_id, m);
    }
    // Add local user SQL row
    let local_sql_member = monotask_core::space::Member {
        pubkey: local_pubkey,
        display_name: if local_profile.display_name.is_empty() { None } else { Some(local_profile.display_name.clone()) },
        avatar_blob: if local_profile.avatar_b64.is_empty() { None } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(&local_profile.avatar_b64).ok()
        },
        bio: None,
        role: None,
        color_accent: None,
        presence: None,
        kicked: false,
    };
    let _ = monotask_storage::space::upsert_member(storage.conn(), &meta.space_id, &local_sql_member);
    // Insert boards (no FK check needed)
    for board_id in &boards_to_insert {
        let _ = monotask_storage::space::add_board(storage.conn(), &meta.space_id, board_id);
    }
    let space = monotask_storage::space::get_space(storage.conn(), &meta.space_id)
        .map_err(|e| e.to_string())?;
    // Auto-dial owner peer addrs embedded in the token so boards sync immediately
    if let Ok(net) = state.net.lock() {
        if let Some(handle) = net.as_ref() {
            for addr in &owner_peer_addrs {
                save_peer_addr(&state.data_dir, addr);
                handle.add_peer_sync(addr.clone());
            }
        }
    }
    drop(storage);
    announce_all_spaces(&state);
    Ok(space_to_view(space))
}

#[tauri::command]
fn add_board_to_space(space_id: String, board_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Update SpaceDoc
    let bytes = monotask_storage::space::load_space_doc(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let mut doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
    monotask_core::space::add_board_ref(&mut doc, &board_id).map_err(|e| e.to_string())?;
    monotask_storage::space::update_space_doc(storage.conn(), &space_id, &doc.save())
        .map_err(|e| e.to_string())?;
    monotask_storage::space::add_board(storage.conn(), &space_id, &board_id)
        .map_err(|e| e.to_string())?;
    drop(storage);
    announce_all_spaces(&state);
    Ok(())
}

#[tauri::command]
fn remove_board_from_space(space_id: String, board_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let bytes = monotask_storage::space::load_space_doc(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let mut doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
    monotask_core::space::remove_board_ref(&mut doc, &board_id).map_err(|e| e.to_string())?;
    monotask_storage::space::update_space_doc(storage.conn(), &space_id, &doc.save())
        .map_err(|e| e.to_string())?;
    monotask_storage::space::remove_board(storage.conn(), &space_id, &board_id)
        .map_err(|e| e.to_string())?;
    // Clean up the board's data and search index entries
    storage.delete_board(&board_id).map_err(|e| e.to_string())?;
    drop(storage);
    announce_all_spaces(&state);
    Ok(())
}

#[tauri::command]
fn kick_member_cmd(space_id: String, pubkey: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let bytes = monotask_storage::space::load_space_doc(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let mut doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
    monotask_core::space::kick_member(&mut doc, &pubkey).map_err(|e| e.to_string())?;
    monotask_storage::space::update_space_doc(storage.conn(), &space_id, &doc.save())
        .map_err(|e| e.to_string())?;
    monotask_storage::space::set_member_kicked(storage.conn(), &space_id, &pubkey, true)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_space_cmd(space_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let my_pubkey = state.identity.public_key_hex();
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let space = monotask_storage::space::get_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    if space.owner_pubkey != my_pubkey {
        return Err("Only the space creator can delete this space".to_string());
    }
    monotask_storage::space::delete_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn leave_space_cmd(space_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let my_pubkey = state.identity.public_key_hex();
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let space = monotask_storage::space::get_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    if space.owner_pubkey == my_pubkey {
        return Err("You are the owner — use Delete Space instead".to_string());
    }
    monotask_storage::space::delete_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_space_cmd(
    space_id: String,
    new_name: String,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    validate_text(&new_name, "Space name", 100)?;
    let my_pubkey = state.identity.public_key_hex();
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let space = monotask_storage::space::get_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    if space.owner_pubkey != my_pubkey {
        return Err("Only the space owner can rename the space".into());
    }
    monotask_storage::space::rename_space(storage.conn(), &space_id, &new_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_my_profile(state: tauri::State<AppState>) -> Result<UserProfileView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let profile = monotask_storage::space::get_profile(storage.conn())
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| monotask_core::space::UserProfile {
            pubkey: state.identity.public_key_hex(),
            display_name: None,
            avatar_blob: None,
            bio: None,
            role: None,
            color_accent: None,
            presence: None,
            ssh_key_path: None,
        });
    Ok(UserProfileView {
        pubkey: profile.pubkey,
        display_name: profile.display_name,
        avatar_b64: profile.avatar_blob.as_ref().map(|b| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(b)
        }),
        bio: profile.bio,
        role: profile.role,
        color_accent: profile.color_accent,
        presence: profile.presence,
        ssh_key_path: profile.ssh_key_path,
    })
}

// SPEC DEVIATION: avatar_b64 is Option<String> (base64) not Option<Vec<u8>>
// DESIGN DECISION: replaces display_name and avatar atomically; UI must re-send existing avatar_b64 when only updating name
#[tauri::command]
fn update_my_profile(
    display_name: String,
    avatar_b64: Option<String>,
    bio: Option<String>,
    role: Option<String>,
    color_accent: Option<String>,
    presence: Option<String>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    if !display_name.is_empty() {
        validate_text(&display_name, "Display name", 100)?;
    }
    use base64::Engine;
    let avatar_blob = if let Some(b64) = avatar_b64.as_deref().filter(|s| !s.is_empty()) {
        // base64 string for 512KB decoded = ~700KB encoded
        if b64.len() > 700_000 {
            return Err("Avatar is too large (max 512 KB)".into());
        }
        base64::engine::general_purpose::STANDARD.decode(b64).ok()
    } else {
        None
    };
    let pubkey = state.identity.public_key_hex();
    let new_profile = monotask_core::space::UserProfile {
        pubkey: pubkey.clone(),
        display_name: if display_name.is_empty() { None } else { Some(display_name.clone()) },
        avatar_blob: avatar_blob.clone(),
        bio: bio.clone().filter(|s| !s.is_empty()),
        role: role.clone().filter(|s| !s.is_empty()),
        color_accent: color_accent.clone().filter(|s| !s.is_empty()),
        presence: presence.clone().filter(|s| !s.is_empty()),
        ssh_key_path: None, // preserved from existing profile below
    };
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Preserve ssh_key_path from existing profile
    let existing = monotask_storage::space::get_profile(storage.conn()).ok().flatten();
    let final_profile = monotask_core::space::UserProfile {
        ssh_key_path: existing.and_then(|p| p.ssh_key_path),
        ..new_profile
    };
    monotask_storage::space::upsert_profile(storage.conn(), &final_profile)
        .map_err(|e| e.to_string())?;
    // Propagate to all SpaceDocs
    let summaries = monotask_storage::space::list_spaces(storage.conn())
        .map_err(|e| e.to_string())?;
    let member_profile = monotask_core::space::MemberProfile {
        display_name: display_name.clone(),
        avatar_b64: avatar_b64.clone().unwrap_or_default(),
        bio: bio.clone().unwrap_or_default(),
        role: role.clone().unwrap_or_default(),
        color_accent: color_accent.clone().unwrap_or_default(),
        presence: presence.clone().unwrap_or_default(),
        kicked: false,
    };
    for summary in summaries {
        if let Ok(bytes) = monotask_storage::space::load_space_doc(storage.conn(), &summary.id) {
            if let Ok(mut doc) = automerge::AutoCommit::load(&bytes) {
                let _ = monotask_core::space::add_member(&mut doc, &pubkey, &member_profile);
                let _ = monotask_storage::space::update_space_doc(storage.conn(), &summary.id, &doc.save());
            }
        }
        // Update SQL cache
        let sql_member = monotask_core::space::Member {
            pubkey: pubkey.clone(),
            display_name: if display_name.is_empty() { None } else { Some(display_name.clone()) },
            avatar_blob: avatar_blob.clone(),
            bio: bio.clone().filter(|s| !s.is_empty()),
            role: role.clone().filter(|s| !s.is_empty()),
            color_accent: color_accent.clone().filter(|s| !s.is_empty()),
            presence: presence.clone().filter(|s| !s.is_empty()),
            kicked: false,
        };
        let _ = monotask_storage::space::upsert_member(storage.conn(), &summary.id, &sql_member);
    }
    Ok(())
}

#[tauri::command]
fn import_ssh_key(path: Option<String>, state: tauri::State<AppState>) -> Result<String, String> {
    let path_ref = path.as_deref().map(std::path::Path::new);
    let identity = monotask_crypto::import_ssh_identity(path_ref)
        .map_err(|e| e.to_string())?;
    let pubkey = identity.public_key_hex();
    let key_bytes = identity.to_secret_bytes();
    // Persist the imported key bytes (overwrite identity.key)
    std::fs::write(state.data_dir.join("identity.key"), &key_bytes)
        .map_err(|e| e.to_string())?;
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Preserve display_name and avatar_blob from existing profile — only pubkey and ssh_key_path change
    let existing = monotask_storage::space::get_profile(storage.conn()).ok().flatten();
    let updated_profile = monotask_core::space::UserProfile {
        pubkey: pubkey.clone(),
        display_name: existing.as_ref().and_then(|p| p.display_name.clone()),
        avatar_blob: existing.as_ref().and_then(|p| p.avatar_blob.clone()),
        bio: existing.as_ref().and_then(|p| p.bio.clone()),
        role: existing.as_ref().and_then(|p| p.role.clone()),
        color_accent: existing.as_ref().and_then(|p| p.color_accent.clone()),
        presence: existing.as_ref().and_then(|p| p.presence.clone()),
        ssh_key_path: path,
    };
    monotask_storage::space::upsert_profile(storage.conn(), &updated_profile)
        .map_err(|e| e.to_string())?;
    Ok(pubkey)
}

#[tauri::command]
async fn upload_avatar_cmd(
    app: tauri::AppHandle,
) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    use base64::Engine;
    let path = app.dialog()
        .file()
        .add_filter("Image", &["png", "jpg", "jpeg", "gif", "webp"])
        .blocking_pick_file();
    match path {
        Some(p) => {
            let bytes = std::fs::read(p.to_string()).map_err(|e| e.to_string())?;
            Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
        }
        None => Err("cancelled".into()),
    }
}

#[derive(serde::Serialize)]
struct SyncPeerView {
    peer_id: String,
}

#[tauri::command]
fn get_sync_status_cmd(state: tauri::State<AppState>) -> Vec<SyncPeerView> {
    let net = match state.net.lock() { Ok(n) => n, Err(_) => return Vec::new() };
    let peers = net.as_ref().map(|h| h.get_peers_sync()).unwrap_or_default();
    peers.into_iter().map(|peer_id| SyncPeerView { peer_id }).collect()
}

#[tauri::command]
fn get_app_version() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build_ts": env!("BUILD_TS").parse::<u64>().unwrap_or(0),
    })
}

fn peers_file(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("bootstrap_peers.txt")
}

fn load_saved_peers(data_dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(peers_file(data_dir))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn save_peer_addr(data_dir: &std::path::Path, addr: &str) {
    let mut peers = load_saved_peers(data_dir);
    if !peers.contains(&addr.to_string()) {
        peers.push(addr.to_string());
        let _ = std::fs::write(peers_file(data_dir), peers.join("\n") + "\n");
    }
}

#[tauri::command]
fn force_sync_cmd(peer_addr: Option<String>, state: tauri::State<AppState>) -> Result<String, String> {
    let net = state.net.lock().map_err(|e| e.to_string())?;
    if let Some(ref handle) = *net {
        // Dial new peer if provided
        if let Some(addr) = &peer_addr {
            let addr = addr.trim().to_string();
            if !addr.is_empty() {
                save_peer_addr(&state.data_dir, &addr);
                handle.add_peer_sync(addr.clone());
            }
        }
        handle.force_rediscovery_sync();
        Ok("Sync triggered".into())
    } else {
        Err("Network not running".into())
    }
}

#[tauri::command]
fn get_saved_peers_cmd(state: tauri::State<AppState>) -> Vec<String> {
    load_saved_peers(&state.data_dir)
}

#[derive(serde::Serialize)]
struct BoardSyncInfo {
    board_id: String,
    title: String,
    last_modified: i64,
}

#[derive(serde::Serialize)]
struct SyncInfo {
    connected_peers: Vec<String>,
    peer_profiles: Vec<PeerIdentityView>,
    boards: Vec<BoardSyncInfo>,
    local_peer_id: String,
}

#[tauri::command]
fn get_sync_info_cmd(state: tauri::State<AppState>) -> Result<SyncInfo, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;

    // Get boards with timestamps
    let rows = monotask_storage::board::list_boards_with_timestamps(storage.conn())
        .map_err(|e| e.to_string())?;

    let mut boards = Vec::new();
    for (board_id, last_modified) in rows {
        // Load board to get title
        let title = if let Ok(doc) = storage.load_board(&board_id) {
            monotask_core::board::get_board_title(&doc).unwrap_or_else(|_| board_id[..8.min(board_id.len())].to_string())
        } else {
            board_id[..8.min(board_id.len())].to_string()
        };
        boards.push(BoardSyncInfo { board_id, title, last_modified });
    }
    drop(storage);

    // Get connected peers from network
    let net = state.net.lock().map_err(|e| e.to_string())?;
    let connected_peers = if let Some(ref handle) = *net {
        handle.get_peers_sync()
    } else {
        vec![]
    };

    // Cross-reference peer pubkeys with member profiles
    let peer_pubkeys = if let Some(ref handle) = *net {
        handle.get_peer_pubkeys_sync()
    } else {
        std::collections::HashMap::new()
    };
    drop(net);

    let storage2 = state.storage.lock().map_err(|e| e.to_string())?;
    let all_spaces = monotask_storage::space::list_spaces(storage2.conn())
        .map_err(|e| e.to_string())?;
    let mut all_members: std::collections::HashMap<String, monotask_core::space::Member> = std::collections::HashMap::new();
    for summary in &all_spaces {
        if let Ok(space) = monotask_storage::space::get_space(storage2.conn(), &summary.id) {
            for m in space.members {
                all_members.entry(m.pubkey.clone()).or_insert(m);
            }
        }
    }
    drop(storage2);

    let peer_profiles: Vec<PeerIdentityView> = connected_peers.iter().map(|peer_id| {
        let pubkey = peer_pubkeys.get(peer_id).cloned().unwrap_or_default();
        let member = all_members.get(&pubkey);
        PeerIdentityView {
            peer_id: peer_id.clone(),
            pubkey: pubkey.clone(),
            display_name: member.and_then(|m| m.display_name.clone()),
            avatar_b64: member.and_then(|m| m.avatar_blob.as_ref().map(|b| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(b)
            })),
            role: member.and_then(|m| m.role.clone()),
            color_accent: member.and_then(|m| m.color_accent.clone()),
            presence: member.and_then(|m| m.presence.clone()),
        }
    }).collect();

    let local_peer_id = monotask_net::NetworkHandle::peer_id_from_identity(
        state.identity.to_secret_bytes()
    );

    Ok(SyncInfo { connected_peers, peer_profiles, boards, local_peer_id })
}

#[tauri::command]
fn remove_saved_peer_cmd(addr: String, state: tauri::State<AppState>) -> Result<(), String> {
    let mut peers = load_saved_peers(&state.data_dir);
    peers.retain(|p| p != &addr);
    std::fs::write(peers_file(&state.data_dir), peers.join("\n") + "\n")
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Chat commands ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ChatRefInput {
    kind: String,
    id: String,
    label: String,
}

#[tauri::command]
fn send_chat_message_cmd(
    space_id: String,
    text: String,
    refs: Vec<ChatRefInput>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let pubkey = state.identity.public_key_hex();
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = get_or_create_chat_doc(&storage, &space_id)?;
    let msg = monotask_core::chat::ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        author: pubkey,
        text,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        refs: refs.into_iter().map(|r| monotask_core::chat::ChatRef {
            kind: r.kind, id: r.id, label: r.label,
        }).collect(),
    };
    monotask_core::chat::append_message(&mut doc, &msg).map_err(|e| e.to_string())?;
    let bytes = doc.save();
    storage.save_board_bytes(&format!("{space_id}-chat"), &bytes, true).map_err(|e| e.to_string())?;
    drop(storage);
    // Trigger sync
    let net = state.net.lock().map_err(|e| e.to_string())?;
    if let Some(ref handle) = *net {
        handle.trigger_sync_sync(format!("{space_id}-chat"));
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct ChatRefView {
    kind: String,
    id: String,
    label: String,
}

#[derive(serde::Serialize)]
struct ChatMessageView {
    id: String,
    author: String,
    display_name: Option<String>,
    avatar_b64: Option<String>,
    color_accent: Option<String>,
    text: String,
    created_at: u64,
    refs: Vec<ChatRefView>,
}

#[tauri::command]
fn get_chat_messages_cmd(
    space_id: String,
    limit: u32,
    before_ts: Option<u64>,
    state: tauri::State<AppState>,
) -> Result<Vec<ChatMessageView>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let doc = get_or_create_chat_doc(&storage, &space_id)?;
    let msgs = monotask_core::chat::list_messages(&doc, limit as usize, before_ts)
        .map_err(|e| e.to_string())?;

    // Build author → member profile lookup
    let space = monotask_storage::space::get_space(storage.conn(), &space_id).ok();
    let member_map: std::collections::HashMap<String, monotask_core::space::Member> = space
        .map(|s| s.members.into_iter().map(|m| (m.pubkey.clone(), m)).collect())
        .unwrap_or_default();

    let views = msgs.into_iter().map(|m| {
        let member = member_map.get(&m.author);
        ChatMessageView {
            id: m.id,
            author: m.author,
            display_name: member.and_then(|mem| mem.display_name.clone()),
            avatar_b64: member.and_then(|mem| mem.avatar_blob.as_ref().map(|b| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(b)
            })),
            color_accent: member.and_then(|mem| mem.color_accent.clone()),
            text: m.text,
            created_at: m.created_at,
            refs: m.refs.into_iter().map(|r| ChatRefView { kind: r.kind, id: r.id, label: r.label }).collect(),
        }
    }).collect();

    Ok(views)
}

#[tauri::command]
fn delete_chat_message_cmd(
    space_id: String,
    message_id: String,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    use automerge::{ReadDoc, transaction::Transactable};
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = get_or_create_chat_doc(&storage, &space_id)?;
    let (_, list_id) = doc.get(automerge::ROOT, "messages")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "chat missing messages list".to_string())?;
    let len = doc.length(&list_id);
    let mut found = false;
    for i in 0..len {
        if let Ok(Some((_, entry))) = doc.get(&list_id, i) {
            if let Ok(Some(id)) = monotask_core::get_string(&doc, &entry, "id") {
                if id == message_id {
                    doc.put(&entry, "deleted", true).map_err(|e| e.to_string())?;
                    found = true;
                    break;
                }
            }
        }
    }
    if !found {
        return Err(format!("Message not found: {message_id}"));
    }
    let bytes = doc.save();
    storage.save_board_bytes(&format!("{space_id}-chat"), &bytes, true)
        .map_err(|e| e.to_string())?;
    drop(storage);
    let net = state.net.lock().map_err(|e| e.to_string())?;
    if let Some(ref handle) = *net {
        handle.trigger_sync_sync(format!("{space_id}-chat"));
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct MentionSuggestion {
    kind: String,     // "member" | "card" | "board"
    id: String,
    label: String,
    sublabel: Option<String>,   // role for members, column for cards
    avatar_b64: Option<String>,
    color_accent: Option<String>,
}

#[tauri::command]
fn get_mention_suggestions_cmd(
    space_id: String,
    query: String,
    kind: String,   // "all" | "member" | "card" | "board"
    state: tauri::State<AppState>,
) -> Result<Vec<MentionSuggestion>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let q = query.to_lowercase();
    let mut results: Vec<MentionSuggestion> = Vec::new();

    if kind == "all" || kind == "member" {
        if let Ok(space) = monotask_storage::space::get_space(storage.conn(), &space_id) {
            for m in space.members.iter().filter(|m| !m.kicked) {
                let name = m.display_name.as_deref().unwrap_or(&m.pubkey);
                if q.is_empty() || name.to_lowercase().contains(&q) {
                    results.push(MentionSuggestion {
                        kind: "member".into(),
                        id: m.pubkey.clone(),
                        label: name.to_string(),
                        sublabel: m.role.clone(),
                        avatar_b64: m.avatar_blob.as_ref().map(|b| {
                            use base64::Engine;
                            base64::engine::general_purpose::STANDARD.encode(b)
                        }),
                        color_accent: m.color_accent.clone(),
                    });
                }
            }
        }
    }

    if kind == "all" || kind == "board" {
        let mut stmt = storage.conn().prepare(
            "SELECT sb.board_id FROM space_boards sb
             JOIN boards b ON b.board_id = sb.board_id
             WHERE sb.space_id = ?1 AND b.is_system = 0"
        ).map_err(|e| e.to_string())?;
        let board_ids: Vec<String> = stmt.query_map([&space_id], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        for board_id in board_ids {
            if let Ok(doc) = storage.load_board(&board_id) {
                let title = monotask_core::board::get_board_title(&doc)
                    .unwrap_or_else(|_| board_id[..8.min(board_id.len())].to_string());
                if q.is_empty() || title.to_lowercase().contains(&q) {
                    results.push(MentionSuggestion {
                        kind: "board".into(),
                        id: board_id,
                        label: title,
                        sublabel: None,
                        avatar_b64: None,
                        color_accent: None,
                    });
                }
            }
        }
    }

    if kind == "all" || kind == "card" {
        let like = format!("%{q}%");
        let mut stmt = storage.conn().prepare(
            "SELECT card_id, board_id, title, column_name FROM card_search_index
             WHERE space_id = ?1 AND (title LIKE ?2 OR ?2 = '%%')
             LIMIT 20"
        ).map_err(|e| e.to_string())?;
        let rows: Vec<(String, String, String, String)> = stmt.query_map(
            rusqlite::params![space_id, like],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        ).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
        for (card_id, _board_id, title, column_name) in rows {
            results.push(MentionSuggestion {
                kind: "card".into(),
                id: card_id,
                label: title,
                sublabel: Some(column_name),
                avatar_b64: None,
                color_accent: None,
            });
        }
    }

    Ok(results)
}

#[derive(serde::Serialize)]
struct SearchResult {
    card_id: String,
    board_id: String,
    title: String,
    column_name: String,
    space_id: String,
}

#[tauri::command]
fn search_cards_cmd(
    query: String,
    state: tauri::State<AppState>,
) -> Result<Vec<SearchResult>, String> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let like = format!("%{}%", query.trim());
    let mut stmt = storage.conn().prepare(
        "SELECT card_id, board_id, title, column_name, space_id
         FROM card_search_index
         WHERE title LIKE ?1
         LIMIT 30"
    ).map_err(|e| e.to_string())?;
    let results = stmt.query_map(
        rusqlite::params![like],
        |row| Ok(SearchResult {
            card_id: row.get(0)?,
            board_id: row.get(1)?,
            title: row.get(2)?,
            column_name: row.get(3)?,
            space_id: row.get(4)?,
        })
    ).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();
    Ok(results)
}

#[tauri::command]
fn find_card_board_cmd(space_id: String, card_id: String, state: tauri::State<AppState>) -> Result<Option<String>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let result = storage.conn().query_row(
        "SELECT board_id FROM card_search_index WHERE card_id = ?1 AND space_id = ?2",
        rusqlite::params![card_id, space_id],
        |row| row.get::<_, String>(0),
    );
    match result {
        Ok(board_id) => Ok(Some(board_id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Use the same data directory as the CLI so they share one database.
            // CLI uses dirs::data_dir().join("p2p-kanban").
            let base_data_dir = dirs::data_dir().expect("failed to resolve data dir");
            // Migrate data from old "p2p-kanban" directory if "monotask" dir doesn't exist yet
            let old_data = base_data_dir.join("p2p-kanban");
            let data_dir = base_data_dir.join("monotask");
            if !data_dir.exists() && old_data.exists() {
                let _ = std::fs::rename(&old_data, &data_dir);
            }
            std::fs::create_dir_all(&data_dir)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
            let _ = app; // suppress unused warning

            let storage = Storage::open(&data_dir)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            let identity = load_identity(&data_dir, storage.conn())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            // Start P2P sync in background
            let identity_bytes = identity.to_secret_bytes();
            let net_storage = Arc::new(Mutex::new(
                Storage::open(&data_dir)
                    .unwrap_or_else(|_| panic!("failed to open storage for net"))
            ));
            let net_config = monotask_net::NetConfig {
                listen_port: 7272,
                data_dir: data_dir.clone(),
                bootstrap_peers: load_saved_peers(&data_dir),
            };
            let mut net_handle = tauri::async_runtime::block_on(
                monotask_net::NetworkHandle::start(net_config, net_storage, identity_bytes)
            ).ok();

            // Drain P2P network events and emit to frontend as Tauri events
            let app_handle_for_events = app.app_handle().clone();
            if let Some(ref mut handle) = net_handle {
                if let Some(mut event_rx) = handle.take_event_rx() {
                    tauri::async_runtime::spawn(async move {
                        while let Some(event) = event_rx.recv().await {
                            match event {
                                monotask_net::NetEvent::BoardSynced { board_id, peer_id } => {
                                    let _ = app_handle_for_events.emit("board-synced",
                                        serde_json::json!({"board_id": board_id, "peer_id": peer_id}));
                                }
                                monotask_net::NetEvent::PeerConnected { peer_id } => {
                                    let _ = app_handle_for_events.emit("peer-connected",
                                        serde_json::json!({"peer_id": peer_id}));
                                }
                                monotask_net::NetEvent::PeerDisconnected { peer_id } => {
                                    let _ = app_handle_for_events.emit("peer-disconnected",
                                        serde_json::json!({"peer_id": peer_id}));
                                }
                                monotask_net::NetEvent::SyncError { board_id, error } => {
                                    let _ = app_handle_for_events.emit("sync-error",
                                        serde_json::json!({"board_id": board_id, "error": error}));
                                }
                            }
                        }
                    });
                }
            }

            // Announce existing spaces so peers can find us immediately on startup.
            if let Some(ref handle) = net_handle {
                let space_ids = monotask_storage::space::list_spaces(storage.conn())
                    .map(|v| v.into_iter().map(|s| s.id).collect::<Vec<_>>())
                    .unwrap_or_default();
                if !space_ids.is_empty() {
                    handle.announce_spaces_sync(space_ids);
                }
            }

            app.manage(AppState {
                storage: Mutex::new(storage),
                identity,
                data_dir: data_dir.clone(),
                net: Mutex::new(net_handle),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_board_cmd,
            list_boards,
            move_card_cmd,
            reorder_card_cmd,
            get_board_detail,
            create_column_cmd,
            rename_board_cmd,
            rename_column_cmd,
            create_card_cmd,
            get_card_cmd,
            update_card_cmd,
            delete_card_cmd,
            delete_column_cmd,
            add_comment_cmd,
            delete_comment_cmd,
            add_checklist_cmd,
            add_checklist_item_cmd,
            toggle_checklist_item_cmd,
            delete_checklist_item_cmd,
            create_space,
            list_spaces,
            get_space_cmd,
            generate_invite,
            revoke_invite,
            export_invite_file,
            get_invite_qr,
            import_invite,
            add_board_to_space,
            remove_board_from_space,
            kick_member_cmd,
            delete_space_cmd,
            leave_space_cmd,
            rename_space_cmd,
            get_my_profile,
            update_my_profile,
            import_ssh_key,
            upload_avatar_cmd,
            get_sync_status_cmd,
            get_app_version,
            force_sync_cmd,
            get_saved_peers_cmd,
            remove_saved_peer_cmd,
            get_sync_info_cmd,
            send_chat_message_cmd,
            get_chat_messages_cmd,
            delete_chat_message_cmd,
            edit_comment_cmd,
            get_mention_suggestions_cmd,
            find_card_board_cmd,
            search_cards_cmd,
            check_for_update_cmd,
            install_update_cmd,
            undo_cmd,
            redo_cmd,
            get_card_history_cmd,
            export_board_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ── Auto-update ─────────────────────────────────────────────────────────────

fn version_is_newer(remote: &str, local: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.').filter_map(|p| p.parse().ok()).collect()
    };
    parse(remote) > parse(local)
}

#[tauri::command]
async fn check_for_update_cmd() -> Result<Option<String>, String> {
    let current = env!("CARGO_PKG_VERSION");
    let output = std::process::Command::new("curl")
        .args([
            "-sf", "--max-time", "10",
            "-H", "User-Agent: monotask-updater",
            "https://api.github.com/repos/nokhodian/monotask/releases/latest",
        ])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() { return Ok(None); }

    let body = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tag = json["tag_name"].as_str().unwrap_or("").trim_start_matches('v').to_string();

    if tag.is_empty() { return Ok(None); }
    if version_is_newer(&tag, current) { Ok(Some(tag)) } else { Ok(None) }
}

#[tauri::command]
async fn install_update_cmd(app: tauri::AppHandle) -> Result<(), String> {
    // Step 1: Fetch the latest release metadata from GitHub.
    let api_out = tokio::process::Command::new("curl")
        .args([
            "-sfL", "--max-time", "15",
            "-H", "User-Agent: monotask-updater",
            "-H", "Accept: application/vnd.github.v3+json",
            "https://api.github.com/repos/nokhodian/monotask/releases/latest",
        ])
        .output().await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    if !api_out.status.success() {
        return Err("GitHub API returned an error".into());
    }

    let body = String::from_utf8_lossy(&api_out.stdout);
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse release JSON: {e}"))?;

    let tag = json["tag_name"].as_str().unwrap_or("latest").to_string();

    #[cfg(target_os = "macos")]
    {
        // Find the aarch64 DMG asset
        let dmg_url = json["assets"].as_array()
            .and_then(|assets| {
                assets.iter().find(|a| {
                    a["name"].as_str()
                        .map(|n| n.ends_with(".dmg") && n.contains("aarch64"))
                        .unwrap_or(false)
                })
            })
            .and_then(|a| a["browser_download_url"].as_str())
            .ok_or("No aarch64 DMG asset found in the release")?
            .to_string();

        // Step 2: Download the DMG
        let tmp_dmg = format!("/tmp/Monotask-{}.dmg", tag);
        let dl = tokio::process::Command::new("curl")
            .args(["-sfL", "--max-time", "300", "-o", &tmp_dmg,
                   "-H", "User-Agent: monotask-updater", &dmg_url])
            .status().await
            .map_err(|e| format!("Download failed: {e}"))?;

        if !dl.success() {
            return Err("Failed to download DMG — check network connection".into());
        }

        // Step 3: Mount the DMG
        let mount_out = tokio::process::Command::new("hdiutil")
            .args(["attach", "-nobrowse", &tmp_dmg])
            .output().await
            .map_err(|e| format!("hdiutil attach failed: {e}"))?;

        if !mount_out.status.success() {
            let _ = tokio::fs::remove_file(&tmp_dmg).await;
            let stderr = String::from_utf8_lossy(&mount_out.stderr).to_string();
            return Err(format!("Failed to mount DMG: {}", stderr.trim()));
        }

        // Find the mount point by scanning /Volumes/ for a directory containing Monotask.app.
        // This is more reliable than parsing hdiutil stdout (which varies with -quiet flag).
        let mount_point = {
            let mut found = None;
            if let Ok(entries) = std::fs::read_dir("/Volumes") {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.join("Monotask.app").exists() {
                        found = Some(p.to_string_lossy().to_string());
                        break;
                    }
                }
            }
            found.ok_or_else(|| "DMG mounted but Monotask.app not found in /Volumes".to_string())?
        };

        // Step 4: Remove old bundle then copy new one.
        // Removing first avoids permission issues with cp overwriting a locked bundle.
        let _ = tokio::process::Command::new("rm")
            .args(["-rf", "/Applications/Monotask.app"])
            .status().await;

        let cp = tokio::process::Command::new("cp")
            .args(["-r", &format!("{}/Monotask.app", mount_point), "/Applications/Monotask.app"])
            .status().await
            .map_err(|e| format!("cp failed: {e}"))?;

        // Step 5: Unmount and clean up
        let _ = tokio::process::Command::new("hdiutil")
            .args(["detach", "-quiet", &mount_point])
            .status().await;
        let _ = tokio::fs::remove_file(&tmp_dmg).await;

        if !cp.success() {
            return Err("Failed to copy Monotask.app to /Applications — you may need to drag-install manually from the DMG on the releases page.".into());
        }

        // Step 6: Clear Gatekeeper quarantine and ad-hoc sign
        let _ = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("find /Applications/Monotask.app -print0 | xargs -0 xattr -c 2>/dev/null; codesign --force --deep --sign - /Applications/Monotask.app 2>/dev/null")
            .status().await;

        // Step 7: Relaunch and exit
        let _ = tokio::process::Command::new("open")
            .args(["-n", "/Applications/Monotask.app"])
            .spawn();

        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        app.exit(0);
    }

    #[cfg(target_os = "windows")]
    {
        // Find the x64 NSIS installer asset
        let exe_url = json["assets"].as_array()
            .and_then(|assets| {
                assets.iter().find(|a| {
                    a["name"].as_str()
                        .map(|n| n.ends_with("-setup.exe") && n.contains("x64"))
                        .unwrap_or(false)
                })
            })
            .and_then(|a| a["browser_download_url"].as_str())
            .ok_or("No x64 installer asset found in the release")?
            .to_string();

        // Step 2: Download installer to %TEMP%
        let tmp_exe = format!("{}\\Monotask-{}-setup.exe",
            std::env::var("TEMP").unwrap_or_else(|_| "C:\\Windows\\Temp".into()),
            tag);

        let dl = tokio::process::Command::new("curl")
            .args(["-sfL", "--max-time", "300", "-o", &tmp_exe,
                   "-H", "User-Agent: monotask-updater", &exe_url])
            .status().await
            .map_err(|e| format!("Download failed: {e}"))?;

        if !dl.success() {
            return Err("Failed to download installer — check network connection".into());
        }

        // Step 3: Launch installer silently (/S = NSIS silent mode) and exit.
        // The installer replaces the running app; we exit first to release the lock.
        let _ = tokio::process::Command::new("cmd")
            .args(["/C", "start", "", "/wait", &tmp_exe, "/S"])
            .spawn()
            .map_err(|e| format!("Failed to launch installer: {e}"))?;

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        app.exit(0);
    }

    Ok(())
}
