// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use kanban_storage::Storage;
use kanban_crypto::Identity;
use tauri::Manager;

struct AppState {
    storage: Mutex<Storage>,
    identity: Identity,
    data_dir: std::path::PathBuf,
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
    ssh_key_path: Option<String>,
}

fn load_identity(
    data_dir: &std::path::Path,
    conn: &rusqlite::Connection,
) -> Result<kanban_crypto::Identity, Box<dyn std::error::Error>> {
    use kanban_crypto::Identity;
    use kanban_storage::space as space_store;

    let key_path = data_dir.join("identity.key");

    // Step 1 & 2: If profile row exists and has an SSH key path, try loading it
    if let Some(profile) = space_store::get_profile(conn)? {
        if let Some(ssh_path) = &profile.ssh_key_path {
            let p = std::path::Path::new(ssh_path);
            if p.exists() {
                if let Ok(id) = kanban_crypto::import_ssh_identity(Some(p)) {
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
    let new_profile = kanban_core::space::UserProfile {
        pubkey: id.public_key_hex(),
        display_name: None,
        avatar_blob: None,
        ssh_key_path: None,
    };
    space_store::upsert_profile(conn, &new_profile)?;
    Ok(id)
}

// ── Space helpers ─────────────────────────────────────────────────────────────

fn space_to_view(space: kanban_core::space::Space) -> SpaceView {
    SpaceView {
        id: space.id,
        name: space.name,
        owner_pubkey: space.owner_pubkey,
        members: space.members.into_iter().map(|m| MemberView {
            pubkey: m.pubkey,
            display_name: m.display_name,
            avatar_b64: m.avatar_blob.map(|b| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&b)
            }),
            kicked: m.kicked,
        }).collect(),
        boards: space.boards,
    }
}

fn local_member_profile(state: &AppState) -> kanban_core::space::MemberProfile {
    use kanban_storage::space as sp;
    let storage = state.storage.lock().unwrap();
    let profile = sp::get_profile(storage.conn()).ok().flatten();
    kanban_core::space::MemberProfile {
        display_name: profile.as_ref()
            .and_then(|p| p.display_name.clone())
            .unwrap_or_default(),
        avatar_b64: profile.as_ref()
            .and_then(|p| p.avatar_blob.as_ref())
            .map(|b| { use base64::Engine; base64::engine::general_purpose::STANDARD.encode(b) })
            .unwrap_or_default(),
        kicked: false,
    }
}

// ── Space commands ────────────────────────────────────────────────────────────

#[tauri::command]
fn create_space(name: String, state: tauri::State<AppState>) -> Result<SpaceView, String> {
    let space_id = uuid::Uuid::new_v4().to_string();
    let owner_pubkey = state.identity.public_key_hex();
    let mut doc = kanban_core::space::create_space_doc(&name, &owner_pubkey)
        .map_err(|e| e.to_string())?;
    let profile = local_member_profile(&state);
    kanban_core::space::add_member(&mut doc, &owner_pubkey, &profile)
        .map_err(|e| e.to_string())?;
    let bytes = doc.save();
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    kanban_storage::space::create_space(storage.conn(), &space_id, &name, &owner_pubkey, &bytes)
        .map_err(|e| e.to_string())?;
    // Add owner as SQL member
    let owner_member = kanban_core::space::Member {
        pubkey: owner_pubkey.clone(),
        display_name: if profile.display_name.is_empty() { None } else { Some(profile.display_name.clone()) },
        avatar_blob: if profile.avatar_b64.is_empty() { None } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(&profile.avatar_b64).ok()
        },
        kicked: false,
    };
    kanban_storage::space::upsert_member(storage.conn(), &space_id, &owner_member)
        .map_err(|e| e.to_string())?;
    let space = kanban_storage::space::get_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
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
    let (mut doc, board) = kanban_core::board::create_board(&title, &pk)
        .map_err(|e| e.to_string())?;
    kanban_storage::board::save_board(storage.conn(), &board.id, &mut doc)
        .map_err(|e| e.to_string())?;
    Ok(BoardSummary { id: board.id, title })
}

#[tauri::command]
fn list_boards(state: tauri::State<AppState>) -> Result<Vec<BoardSummary>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let ids = kanban_storage::board::list_board_ids(storage.conn())
        .map_err(|e| e.to_string())?;
    let mut boards = Vec::with_capacity(ids.len());
    for id in ids {
        let title = kanban_storage::board::load_board(storage.conn(), &id)
            .ok()
            .and_then(|doc| kanban_core::board::get_board_title(&doc).ok())
            .unwrap_or_else(|| id.clone());
        boards.push(BoardSummary { id, title });
    }
    Ok(boards)
}

#[tauri::command]
fn get_board_detail(board_id: String, state: tauri::State<AppState>) -> Result<BoardDetail, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = kanban_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let title = kanban_core::board::get_board_title(&doc)
        .unwrap_or_else(|_| board_id.clone());

    // Auto-create default columns if board has none
    let existing = kanban_core::column::list_columns(&doc).unwrap_or_default();
    if existing.is_empty() {
        drop(existing);
        kanban_core::init_doc(&mut doc).map_err(|e| e.to_string())?;
        for col_title in DEFAULT_COLUMNS {
            kanban_core::column::create_column(&mut doc, col_title)
                .map_err(|e| e.to_string())?;
        }
        kanban_storage::board::save_board(storage.conn(), &board_id, &mut doc)
            .map_err(|e| e.to_string())?;
    }

    let columns = kanban_core::column::list_columns(&doc).map_err(|e| e.to_string())?;
    let mut col_views = Vec::with_capacity(columns.len());
    for col in &columns {
        let card_ids = get_column_card_ids(&doc, &col.id);
        let mut cards = Vec::new();
        for cid in card_ids {
            if let Ok(card) = kanban_core::card::read_card(&doc, &cid) {
                if !card.deleted && !card.archived {
                    let labels = get_card_labels(&doc, &cid);
                    let history = get_card_history(&doc, &cid);
                    let last_move = history.into_iter().last();
                    let assignee = get_card_str_field(&doc, &cid, "assignee");
                    cards.push(CardView {
                        id: card.id.clone(),
                        title: card.title,
                        number: card.number.map(|n| n.to_display()),
                        has_description: !card.description.is_empty(),
                        due_date: card.due_date,
                        assignee,
                        labels,
                        last_move,
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
    let card_obj = match kanban_core::card::get_card_obj(doc, card_id) {
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
                let from_col = kanban_core::get_string(doc, &obj, "from_col").ok().flatten().unwrap_or_default();
                let to_col   = kanban_core::get_string(doc, &obj, "to_col").ok().flatten().unwrap_or_default();
                let timestamp = kanban_core::get_string(doc, &obj, "timestamp").ok().flatten().unwrap_or_default();
                Some(MoveEvent { from_col, to_col, timestamp })
            })
        })
        .collect()
}

fn get_card_str_field(doc: &automerge::AutoCommit, card_id: &str, field: &str) -> Option<String> {
    use automerge::ReadDoc;
    let card_obj = kanban_core::card::get_card_obj(doc, card_id).ok()?;
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
    let card_obj = match kanban_core::card::get_card_obj(doc, card_id) {
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
    let col_obj = match kanban_core::column::find_column_obj(doc, col_id) {
        Ok(Some(o)) => o,
        _ => return vec![],
    };
    let card_ids_list = match kanban_core::column::get_card_ids_list(doc, &col_obj) {
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
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = kanban_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let col_id = kanban_core::column::create_column(&mut doc, &title)
        .map_err(|e| e.to_string())?;
    kanban_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    Ok(col_id)
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
    let mut doc = kanban_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;

    // Remove from source column
    {
        let from_obj = kanban_core::column::find_column_obj(&doc, &from_col_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("column not found: {from_col_id}"))?;
        let card_ids = kanban_core::column::get_card_ids_list(&doc, &from_obj)
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
    kanban_core::column::append_card_to_column(&mut doc, &to_col_id, &card_id)
        .map_err(|e| e.to_string())?;

    // Record movement history on the card
    {
        use automerge::{ObjType, transaction::Transactable};
        let columns = kanban_core::column::list_columns(&doc).unwrap_or_default();
        eprintln!("[move_card_cmd] card={card_id} columns_found={}", columns.len());
        let from_title = columns.iter().find(|c| c.id == from_col_id).map(|c| c.title.clone()).unwrap_or_else(|| from_col_id.clone());
        let to_title   = columns.iter().find(|c| c.id == to_col_id).map(|c| c.title.clone()).unwrap_or_else(|| to_col_id.clone());
        eprintln!("[move_card_cmd] from={from_title:?} to={to_title:?}");
        let card_obj = kanban_core::card::get_card_obj(&doc, &card_id).map_err(|e| e.to_string())?;
        let hist_obj = match doc.get(&card_obj, "history").map_err(|e| e.to_string())? {
            Some((_, id)) => { eprintln!("[move_card_cmd] history list exists"); id }
            None => { eprintln!("[move_card_cmd] creating history list"); doc.put_object(&card_obj, "history", ObjType::List).map_err(|e| e.to_string())? }
        };
        let idx = doc.length(&hist_obj);
        eprintln!("[move_card_cmd] inserting history event at idx={idx}");
        let ts = kanban_core::clock::now();
        let ev = doc.insert_object(&hist_obj, idx, ObjType::Map).map_err(|e| e.to_string())?;
        doc.put(&ev, "from_col",  from_title.as_str()).map_err(|e| e.to_string())?;
        doc.put(&ev, "to_col",    to_title.as_str()).map_err(|e| e.to_string())?;
        doc.put(&ev, "timestamp", ts.as_str()).map_err(|e| e.to_string())?;
        eprintln!("[move_card_cmd] history recorded OK");
    }

    kanban_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn create_card_cmd(board_id: String, col_id: String, title: String, state: tauri::State<AppState>) -> Result<String, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = kanban_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let pk = state.identity.public_key_bytes();
    let card = kanban_core::card::create_card(&mut doc, &col_id, &title, &pk, &[pk.to_vec()])
        .map_err(|e| e.to_string())?;
    kanban_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    Ok(card.id)
}

#[tauri::command]
fn get_card_cmd(board_id: String, card_id: String, state: tauri::State<AppState>) -> Result<CardDetailView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let doc = kanban_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let card = kanban_core::card::read_card(&doc, &card_id).map_err(|e| e.to_string())?;
    let labels = get_card_labels(&doc, &card_id);
    let assignee = get_card_str_field(&doc, &card_id, "assignee");
    let history = get_card_history(&doc, &card_id);
    let comments = kanban_core::comment::list_comments(&doc, &card_id)
        .unwrap_or_default()
        .into_iter()
        .map(|c| CommentView { id: c.id, author: c.author, text: c.text, created_at: c.created_at })
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
    state: tauri::State<AppState>,
) -> Result<(), String> {
    use automerge::{ReadDoc, ObjType, transaction::Transactable};
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = kanban_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let card_obj = kanban_core::card::get_card_obj(&doc, &card_id).map_err(|e| e.to_string())?;
    doc.put(&card_obj, "title", title.as_str()).map_err(|e| e.to_string())?;
    doc.put(&card_obj, "description", description.as_str()).map_err(|e| e.to_string())?;
    let due = due_date.as_deref().unwrap_or("");
    doc.put(&card_obj, "due_date", due).map_err(|e| e.to_string())?;
    let assignee_val = assignee.as_deref().unwrap_or("");
    doc.put(&card_obj, "assignee", assignee_val).map_err(|e| e.to_string())?;
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
    kanban_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn add_comment_cmd(board_id: String, card_id: String, text: String, state: tauri::State<AppState>) -> Result<CommentView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = kanban_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let author = state.identity.public_key_hex();
    let comment = kanban_core::comment::add_comment(&mut doc, &card_id, &text, &author)
        .map_err(|e| e.to_string())?;
    kanban_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    Ok(CommentView { id: comment.id, author: comment.author, text: comment.text, created_at: comment.created_at })
}

#[tauri::command]
fn delete_comment_cmd(board_id: String, card_id: String, comment_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = kanban_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    kanban_core::comment::delete_comment(&mut doc, &card_id, &comment_id)
        .map_err(|e| e.to_string())?;
    kanban_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_card_cmd(board_id: String, card_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = kanban_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    kanban_core::card::delete_card(&mut doc, &card_id).map_err(|e| e.to_string())?;
    kanban_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_column_cmd(board_id: String, col_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    use automerge::{ReadDoc, transaction::Transactable};
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = kanban_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    let cols = match doc.get(automerge::ROOT, "columns").map_err(|e| e.to_string())? {
        Some((_, id)) => id,
        None => return Err("board has no columns".into()),
    };
    let len = doc.length(&cols);
    let idx = (0..len).find(|&i| {
        doc.get(&cols, i).ok().flatten()
            .and_then(|(_, obj)| kanban_core::get_string(&doc, &obj, "id").ok().flatten())
            .map(|s| s == col_id)
            .unwrap_or(false)
    }).ok_or_else(|| format!("column not found: {col_id}"))?;
    doc.delete(&cols, idx).map_err(|e| e.to_string())?;
    kanban_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn list_spaces(state: tauri::State<AppState>) -> Result<Vec<SpaceSummaryView>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let summaries = kanban_storage::space::list_spaces(storage.conn())
        .map_err(|e| e.to_string())?;
    Ok(summaries.into_iter().map(|s| SpaceSummaryView {
        id: s.id, name: s.name, member_count: s.member_count,
    }).collect())
}

#[tauri::command]
fn get_space_cmd(space_id: String, state: tauri::State<AppState>) -> Result<SpaceView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let space = kanban_storage::space::get_space(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    Ok(space_to_view(space))
}

#[tauri::command]
fn generate_invite(space_id: String, state: tauri::State<AppState>) -> Result<String, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Revoke any existing active token first
    kanban_storage::space::revoke_all_invites(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let token = kanban_crypto::generate_invite_token(&space_id, &state.identity)
        .map_err(|e| e.to_string())?;
    let meta = kanban_crypto::verify_invite_token_signature(&token)
        .map_err(|e| e.to_string())?;
    kanban_storage::space::insert_invite(storage.conn(), &meta.token_hash, &token, &space_id, None)
        .map_err(|e| e.to_string())?;
    Ok(token)
}

#[tauri::command]
fn revoke_invite(space_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    kanban_storage::space::revoke_all_invites(storage.conn(), &space_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn export_invite_file(space_id: String, path: String, state: tauri::State<AppState>) -> Result<(), String> {
    // Inline token generation (State<AppState> does not implement Clone, so we can't call generate_invite())
    let (token, space_name, doc_bytes) = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        // Revoke existing + generate fresh token
        kanban_storage::space::revoke_all_invites(storage.conn(), &space_id)
            .map_err(|e| e.to_string())?;
        let tok = kanban_crypto::generate_invite_token(&space_id, &state.identity)
            .map_err(|e| e.to_string())?;
        let meta = kanban_crypto::verify_invite_token_signature(&tok)
            .map_err(|e| e.to_string())?;
        kanban_storage::space::insert_invite(storage.conn(), &meta.token_hash, &tok, &space_id, None)
            .map_err(|e| e.to_string())?;
        let space = kanban_storage::space::get_space(storage.conn(), &space_id)
            .map_err(|e| e.to_string())?;
        let bytes = kanban_storage::space::load_space_doc(storage.conn(), &space_id)
            .map_err(|e| e.to_string())?;
        (tok, space.name, bytes)
    };
    use base64::Engine;
    let space_doc_b64 = base64::engine::general_purpose::STANDARD.encode(&doc_bytes);
    let payload = serde_json::json!({
        "token": token,
        "space_name": space_name,
        "space_doc": space_doc_b64,
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
    kanban_storage::space::get_active_invite_token(storage.conn(), &space_id)
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
    let meta = kanban_crypto::verify_invite_token_signature(&token)
        .map_err(|e| e.to_string())?;

    // 3. Check policy (owner-side only)
    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        kanban_storage::space::check_invite_policy(storage.conn(), &meta, &local_pubkey)
            .map_err(|e| e.to_string())?;

        // 4. Idempotency check
        let already = kanban_storage::space::get_space(storage.conn(), &meta.space_id);
        if let Ok(existing) = already {
            // Check if local user is a member
            if existing.members.iter().any(|m| m.pubkey == local_pubkey) {
                return Ok(space_to_view(existing));
            }
        }
    }

    // 5–8. Create or merge space
    let space_name = space_name_hint;
    let (mut doc, members_to_insert, boards_to_insert) = if let Some(bytes) = space_doc_bytes {
        let doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
        let members = kanban_core::space::list_members(&doc).map_err(|e| e.to_string())?;
        let boards = kanban_core::space::list_board_refs(&doc).map_err(|e| e.to_string())?;
        (doc, members, boards)
    } else {
        let mut doc = kanban_core::space::create_space_doc(&space_name, &meta.owner_pubkey)
            .map_err(|e| e.to_string())?;
        let empty = kanban_core::space::MemberProfile {
            display_name: String::new(), avatar_b64: String::new(), kicked: false,
        };
        kanban_core::space::add_member(&mut doc, &meta.owner_pubkey, &empty)
            .map_err(|e| e.to_string())?;
        // Include stub owner so SQL space_members row is created for them
        let stub_owner = kanban_core::space::Member {
            pubkey: meta.owner_pubkey.clone(),
            display_name: None,
            avatar_blob: None,
            kicked: false,
        };
        (doc, vec![stub_owner], vec![])
    };

    // Add local user to SpaceDoc
    let local_profile = local_member_profile(&state);
    kanban_core::space::add_member(&mut doc, &local_pubkey, &local_profile)
        .map_err(|e| e.to_string())?;
    let doc_bytes = doc.save();

    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Create space row (or skip if already exists from idempotency path)
    let _ = kanban_storage::space::create_space(
        storage.conn(), &meta.space_id, &space_name, &meta.owner_pubkey, &doc_bytes,
    );
    // Insert members from snapshot
    for m in &members_to_insert {
        let _ = kanban_storage::space::upsert_member(storage.conn(), &meta.space_id, m);
    }
    // Add local user SQL row
    let local_sql_member = kanban_core::space::Member {
        pubkey: local_pubkey,
        display_name: if local_profile.display_name.is_empty() { None } else { Some(local_profile.display_name.clone()) },
        avatar_blob: if local_profile.avatar_b64.is_empty() { None } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(&local_profile.avatar_b64).ok()
        },
        kicked: false,
    };
    let _ = kanban_storage::space::upsert_member(storage.conn(), &meta.space_id, &local_sql_member);
    // Insert boards (no FK check needed)
    for board_id in &boards_to_insert {
        let _ = kanban_storage::space::add_board(storage.conn(), &meta.space_id, board_id);
    }
    let space = kanban_storage::space::get_space(storage.conn(), &meta.space_id)
        .map_err(|e| e.to_string())?;
    Ok(space_to_view(space))
}

#[tauri::command]
fn add_board_to_space(space_id: String, board_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Update SpaceDoc
    let bytes = kanban_storage::space::load_space_doc(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let mut doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
    kanban_core::space::add_board_ref(&mut doc, &board_id).map_err(|e| e.to_string())?;
    kanban_storage::space::update_space_doc(storage.conn(), &space_id, &doc.save())
        .map_err(|e| e.to_string())?;
    kanban_storage::space::add_board(storage.conn(), &space_id, &board_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_board_from_space(space_id: String, board_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let bytes = kanban_storage::space::load_space_doc(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let mut doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
    kanban_core::space::remove_board_ref(&mut doc, &board_id).map_err(|e| e.to_string())?;
    kanban_storage::space::update_space_doc(storage.conn(), &space_id, &doc.save())
        .map_err(|e| e.to_string())?;
    kanban_storage::space::remove_board(storage.conn(), &space_id, &board_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn kick_member_cmd(space_id: String, pubkey: String, state: tauri::State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let bytes = kanban_storage::space::load_space_doc(storage.conn(), &space_id)
        .map_err(|e| e.to_string())?;
    let mut doc = automerge::AutoCommit::load(&bytes).map_err(|e| e.to_string())?;
    kanban_core::space::kick_member(&mut doc, &pubkey).map_err(|e| e.to_string())?;
    kanban_storage::space::update_space_doc(storage.conn(), &space_id, &doc.save())
        .map_err(|e| e.to_string())?;
    kanban_storage::space::set_member_kicked(storage.conn(), &space_id, &pubkey, true)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_my_profile(state: tauri::State<AppState>) -> Result<UserProfileView, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let profile = kanban_storage::space::get_profile(storage.conn())
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| kanban_core::space::UserProfile {
            pubkey: state.identity.public_key_hex(),
            display_name: None,
            avatar_blob: None,
            ssh_key_path: None,
        });
    Ok(UserProfileView {
        pubkey: profile.pubkey,
        display_name: profile.display_name,
        avatar_b64: profile.avatar_blob.map(|b| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(&b)
        }),
        ssh_key_path: profile.ssh_key_path,
    })
}

// SPEC DEVIATION: avatar_b64 is Option<String> (base64) not Option<Vec<u8>>
// DESIGN DECISION: replaces display_name and avatar atomically; UI must re-send existing avatar_b64 when only updating name
#[tauri::command]
fn update_my_profile(
    display_name: String,
    avatar_b64: Option<String>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    use base64::Engine;
    let avatar_blob = avatar_b64.as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok());
    let pubkey = state.identity.public_key_hex();
    let new_profile = kanban_core::space::UserProfile {
        pubkey: pubkey.clone(),
        display_name: if display_name.is_empty() { None } else { Some(display_name.clone()) },
        avatar_blob: avatar_blob.clone(),
        ssh_key_path: None, // preserved from existing profile below
    };
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Preserve ssh_key_path from existing profile
    let existing = kanban_storage::space::get_profile(storage.conn()).ok().flatten();
    let final_profile = kanban_core::space::UserProfile {
        ssh_key_path: existing.and_then(|p| p.ssh_key_path),
        ..new_profile
    };
    kanban_storage::space::upsert_profile(storage.conn(), &final_profile)
        .map_err(|e| e.to_string())?;
    // Propagate to all SpaceDocs
    let summaries = kanban_storage::space::list_spaces(storage.conn())
        .map_err(|e| e.to_string())?;
    let member_profile = kanban_core::space::MemberProfile {
        display_name: display_name.clone(),
        avatar_b64: avatar_b64.clone().unwrap_or_default(),
        kicked: false,
    };
    for summary in summaries {
        if let Ok(bytes) = kanban_storage::space::load_space_doc(storage.conn(), &summary.id) {
            if let Ok(mut doc) = automerge::AutoCommit::load(&bytes) {
                let _ = kanban_core::space::add_member(&mut doc, &pubkey, &member_profile);
                let _ = kanban_storage::space::update_space_doc(storage.conn(), &summary.id, &doc.save());
            }
        }
        // Update SQL cache
        let sql_member = kanban_core::space::Member {
            pubkey: pubkey.clone(),
            display_name: if display_name.is_empty() { None } else { Some(display_name.clone()) },
            avatar_blob: avatar_blob.clone(),
            kicked: false,
        };
        let _ = kanban_storage::space::upsert_member(storage.conn(), &summary.id, &sql_member);
    }
    Ok(())
}

#[tauri::command]
fn import_ssh_key(path: Option<String>, state: tauri::State<AppState>) -> Result<String, String> {
    let path_ref = path.as_deref().map(std::path::Path::new);
    let identity = kanban_crypto::import_ssh_identity(path_ref)
        .map_err(|e| e.to_string())?;
    let pubkey = identity.public_key_hex();
    let key_bytes = identity.to_secret_bytes();
    // Persist the imported key bytes (overwrite identity.key)
    std::fs::write(state.data_dir.join("identity.key"), &key_bytes)
        .map_err(|e| e.to_string())?;
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    // Preserve display_name and avatar_blob from existing profile — only pubkey and ssh_key_path change
    let existing = kanban_storage::space::get_profile(storage.conn()).ok().flatten();
    let updated_profile = kanban_core::space::UserProfile {
        pubkey: pubkey.clone(),
        display_name: existing.as_ref().and_then(|p| p.display_name.clone()),
        avatar_blob: existing.and_then(|p| p.avatar_blob),
        ssh_key_path: path,
    };
    kanban_storage::space::upsert_profile(storage.conn(), &updated_profile)
        .map_err(|e| e.to_string())?;
    Ok(pubkey)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // Use the same data directory as the CLI so they share one database.
            // CLI uses dirs::data_dir().join("p2p-kanban").
            let data_dir = dirs::data_dir()
                .expect("failed to resolve data dir")
                .join("p2p-kanban");
            std::fs::create_dir_all(&data_dir)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
            let _ = app; // suppress unused warning

            let storage = Storage::open(&data_dir)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            let identity = load_identity(&data_dir, storage.conn())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            app.manage(AppState {
                storage: Mutex::new(storage),
                identity,
                data_dir: data_dir.clone(),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_board_cmd,
            list_boards,
            move_card_cmd,
            get_board_detail,
            create_column_cmd,
            create_card_cmd,
            get_card_cmd,
            update_card_cmd,
            delete_card_cmd,
            delete_column_cmd,
            add_comment_cmd,
            delete_comment_cmd,
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
            get_my_profile,
            update_my_profile,
            import_ssh_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
