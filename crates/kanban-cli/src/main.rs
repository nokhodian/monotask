use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "monotask", about = "Monotask – P2P task manager CLI")]
struct Cli {
    #[arg(long, global = true, help = "Data directory")]
    data_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize identity and config
    Init,
    /// Board management
    Board {
        #[command(subcommand)]
        cmd: BoardCommands,
    },
    /// Column management
    Column {
        #[command(subcommand)]
        cmd: ColumnCommands,
    },
    /// Card management
    Card {
        #[command(subcommand)]
        cmd: CardCommands,
    },
    /// Checklist management
    Checklist {
        #[command(subcommand)]
        cmd: ChecklistCommands,
    },
    /// Manage Spaces (shared containers for boards)
    Space {
        #[command(subcommand)]
        cmd: SpaceCommands,
    },
    /// Manage your local identity and profile
    Profile {
        #[command(subcommand)]
        cmd: ProfileCommands,
    },
    /// Print full reference documentation for AI agents and automation
    #[command(name = "ai-help")]
    AiHelp,
    /// Start P2P sync daemon
    Sync {
        /// Run in background (writes PID to data dir)
        #[arg(long)]
        detach: bool,
        /// Stop background daemon
        #[arg(long)]
        stop: bool,
        /// Show sync status
        #[arg(long)]
        status: bool,
        /// TCP port to listen on (default: OS-assigned). Use a fixed port when
        /// peers need to dial you directly via --peer.
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Dial a specific peer at startup (bypasses mDNS). Format:
        /// /ip4/1.2.3.4/tcp/7272  — repeat for multiple peers.
        #[arg(long = "peer")]
        peers: Vec<String>,
    },
}

#[derive(Subcommand)]
enum BoardCommands {
    Create { title: String, #[arg(long)] json: bool },
    List { #[arg(long)] json: bool },
}

#[derive(Subcommand)]
enum ColumnCommands {
    Create { board_id: String, title: String, #[arg(long)] json: bool },
    List { board_id: String, #[arg(long)] json: bool },
}

#[derive(Subcommand)]
enum CardCommands {
    Create { board_id: String, col_id: String, title: String, #[arg(long)] json: bool },
    View { board_id: String, card_id: String, #[arg(long)] json: bool },
    /// Comment management
    Comment {
        #[command(subcommand)]
        cmd: CommentCommands,
    },
}

#[derive(Subcommand)]
enum CommentCommands {
    /// Add a comment to a card
    Add {
        board_id: String,
        card_id: String,
        text: String,
        #[arg(long)] json: bool,
    },
    /// List comments on a card
    List {
        board_id: String,
        card_id: String,
        #[arg(long)] json: bool,
    },
    /// Delete a comment
    Delete {
        board_id: String,
        card_id: String,
        comment_id: String,
        #[arg(long)] json: bool,
    },
}

#[derive(Subcommand)]
enum ChecklistCommands {
    /// Add a checklist to a card
    Add {
        board_id: String,
        card_id: String,
        title: String,
        #[arg(long)] json: bool,
    },
    /// Add an item to a checklist
    ItemAdd {
        board_id: String,
        card_id: String,
        checklist_id: String,
        text: String,
        #[arg(long)] json: bool,
    },
    /// Check a checklist item
    ItemCheck {
        board_id: String,
        card_id: String,
        checklist_id: String,
        item_id: String,
        #[arg(long)] json: bool,
    },
    /// Uncheck a checklist item
    ItemUncheck {
        board_id: String,
        card_id: String,
        checklist_id: String,
        item_id: String,
        #[arg(long)] json: bool,
    },
}

#[derive(clap::Subcommand)]
enum SpaceCommands {
    /// Create a new Space
    Create { name: String },
    /// List all local Spaces
    List,
    /// Show details of a Space
    Info { space_id: String },
    Invite {
        #[command(subcommand)]
        cmd: SpaceInviteCommands,
    },
    /// Join a Space via a token or .space file
    Join { token_or_file: String },
    Boards {
        #[command(subcommand)]
        cmd: SpaceBoardsCommands,
    },
    Members {
        #[command(subcommand)]
        cmd: SpaceMembersCommands,
    },
}

#[derive(clap::Subcommand)]
enum SpaceInviteCommands {
    /// Generate a new invite token for a Space
    Generate { space_id: String },
    /// Export an invite as a .space file
    Export { space_id: String, output_file: String },
    /// Revoke all active invites for a Space
    Revoke { space_id: String },
}

#[derive(clap::Subcommand)]
enum SpaceBoardsCommands {
    /// Add a board to a Space
    Add { space_id: String, board_id: String },
    /// Remove a board from a Space
    Remove { space_id: String, board_id: String },
    /// List boards in a Space
    List { space_id: String },
}

#[derive(clap::Subcommand)]
enum SpaceMembersCommands {
    /// List members of a Space
    List { space_id: String },
    /// Kick a member from a Space
    Kick { space_id: String, pubkey: String },
}

#[derive(clap::Subcommand)]
enum ProfileCommands {
    /// Show your current profile
    Show,
    /// Set your display name
    SetName { name: String },
    /// Set your avatar from an image file
    SetAvatar { path: String },
    /// Import an SSH Ed25519 key as your identity
    ImportSshKey { path: Option<String> },
}

fn data_dir(cli: &Cli) -> anyhow::Result<std::path::PathBuf> {
    if let Some(d) = &cli.data_dir {
        return Ok(d.clone());
    }
    let base = dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!(
            "Cannot determine data directory. Use --data-dir to specify one explicitly."
        ))?;
    // Migrate data from old "p2p-kanban" directory if new "monotask" dir doesn't exist yet
    let new_dir = base.join("monotask");
    let old_dir = base.join("p2p-kanban");
    if !new_dir.exists() && old_dir.exists() {
        let _ = std::fs::rename(&old_dir, &new_dir);
    }
    Ok(new_dir)
}

fn load_cli_identity(data_dir: &std::path::Path, conn: &rusqlite::Connection) -> anyhow::Result<kanban_crypto::Identity> {
    use kanban_crypto::Identity;
    use kanban_storage::space as space_store;
    let key_path = data_dir.join("identity.key");
    // Step 1: Try SSH key from profile
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
    // Step 2: Fall back to identity.key
    if key_path.exists() {
        let bytes = std::fs::read(&key_path)?;
        if bytes.len() == 32 {
            let arr: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("bad key len"))?;
            return Ok(Identity::from_secret_bytes(&arr));
        }
    }
    // Step 3: Generate new identity
    let id = Identity::generate();
    std::fs::write(&key_path, id.to_secret_bytes())?;
    let new_profile = kanban_core::space::UserProfile {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let dir = data_dir(&cli)?;
    let mut storage = kanban_storage::Storage::open(&dir)?;
    let identity = load_cli_identity(&dir, storage.conn())?;

    match cli.command {
        Commands::Init => {
            println!("Initialized monotask at {}", dir.display());
        }
        Commands::Board { cmd } => match cmd {
            BoardCommands::Create { title, json } => {
                let id = kanban_crypto::Identity::generate();
                let (mut doc, board) = kanban_core::board::create_board(&title, &id.public_key_hex())?;
                storage.save_board(&board.id, &mut doc)?;
                if json {
                    let deep_link = format!("monotask://board/{}", board.id);
                    println!("{}", serde_json::json!({"id": board.id, "title": board.title, "deep_link": deep_link}));
                } else {
                    println!("Created board: {} ({})", board.title, board.id);
                }
            }
            BoardCommands::List { json } => {
                let ids = storage.list_board_ids()?;
                if json { println!("{}", serde_json::to_string_pretty(&ids)?); }
                else { for id in &ids { println!("{id}"); } }
            }
        },
        Commands::Column { cmd } => match cmd {
            ColumnCommands::Create { board_id, title, json } => {
                let mut doc = storage.load_board(&board_id)?;
                let col_id = kanban_core::column::create_column(&mut doc, &title)?;
                storage.save_board(&board_id, &mut doc)?;
                if json { println!("{}", serde_json::json!({"id": col_id, "board_id": board_id})); }
                else { println!("Created column: {title} ({col_id})"); }
            }
            ColumnCommands::List { board_id, json } => {
                let doc = storage.load_board(&board_id)?;
                let cols = kanban_core::column::list_columns(&doc)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&cols)?);
                } else {
                    for col in &cols {
                        println!("{}: {}", col.id, col.title);
                    }
                }
            }
        },
        Commands::Card { cmd } => match cmd {
            CardCommands::Create { board_id, col_id, title, json } => {
                let mut doc = storage.load_board(&board_id)?;
                // Placeholder until identity system is wired in Phase 3
                let actor_pk = vec![0u8; 32];
                let members = vec![actor_pk.clone()];
                let card = kanban_core::card::create_card(&mut doc, &col_id, &title, &actor_pk, &members)?;
                storage.save_board(&board_id, &mut doc)?;
                if json {
                    let number_display = card.number.as_ref().map(|n| n.to_display());
                    println!("{}", serde_json::json!({"id": card.id, "title": card.title, "board_id": board_id, "number": number_display}));
                } else {
                    println!("Created card: {} ({})", card.title, card.id);
                }
            }
            CardCommands::View { board_id, card_id, json } => {
                let doc = storage.load_board(&board_id)?;
                let card = kanban_core::card::read_card(&doc, &card_id)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&card)?);
                } else {
                    println!("ID:          {}", card.id);
                    println!("Title:       {}", card.title);
                    if !card.description.is_empty() {
                        println!("Description: {}", card.description);
                    }
                    if card.deleted { println!("Status:      DELETED"); }
                    else if card.archived { println!("Status:      archived"); }
                    if let Some(due) = &card.due_date { println!("Due:         {due}"); }
                }
            }
            CardCommands::Comment { cmd } => match cmd {
                CommentCommands::Add { board_id, card_id, text, json } => {
                    let mut doc = storage.load_board(&board_id)?;
                    // Use placeholder identity until Phase 3 wires real identity
                    let author_key = "placeholder";
                    let comment = kanban_core::comment::add_comment(&mut doc, &card_id, &text, author_key)?;
                    storage.save_board(&board_id, &mut doc)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&comment)?);
                    } else {
                        println!("Added comment {}", comment.id);
                    }
                }
                CommentCommands::List { board_id, card_id, json } => {
                    let doc = storage.load_board(&board_id)?;
                    let comments = kanban_core::comment::list_comments(&doc, &card_id)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&comments)?);
                    } else {
                        for c in &comments {
                            println!("[{}] {}: {}", c.created_at, c.author, c.text);
                        }
                    }
                }
                CommentCommands::Delete { board_id, card_id, comment_id, json } => {
                    let mut doc = storage.load_board(&board_id)?;
                    kanban_core::comment::delete_comment(&mut doc, &card_id, &comment_id)?;
                    storage.save_board(&board_id, &mut doc)?;
                    if json {
                        println!("{}", serde_json::json!({"deleted": comment_id}));
                    } else {
                        println!("Deleted comment {comment_id}");
                    }
                }
            },
        },
        Commands::Checklist { cmd } => match cmd {
            ChecklistCommands::Add { board_id, card_id, title, json } => {
                let mut doc = storage.load_board(&board_id)?;
                let cl = kanban_core::checklist::add_checklist(&mut doc, &card_id, &title)?;
                storage.save_board(&board_id, &mut doc)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&cl)?);
                } else {
                    println!("Created checklist: {} ({})", cl.title, cl.id);
                }
            }
            ChecklistCommands::ItemAdd { board_id, card_id, checklist_id, text, json } => {
                let mut doc = storage.load_board(&board_id)?;
                let item = kanban_core::checklist::add_checklist_item(&mut doc, &card_id, &checklist_id, &text)?;
                storage.save_board(&board_id, &mut doc)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&item)?);
                } else {
                    println!("Added item: {} ({})", item.text, item.id);
                }
            }
            ChecklistCommands::ItemCheck { board_id, card_id, checklist_id, item_id, json } => {
                let mut doc = storage.load_board(&board_id)?;
                kanban_core::checklist::set_item_checked(&mut doc, &card_id, &checklist_id, &item_id, true)?;
                storage.save_board(&board_id, &mut doc)?;
                if json {
                    println!("{}", serde_json::json!({"checked": true, "item_id": item_id}));
                } else {
                    println!("Checked item {item_id}");
                }
            }
            ChecklistCommands::ItemUncheck { board_id, card_id, checklist_id, item_id, json } => {
                let mut doc = storage.load_board(&board_id)?;
                kanban_core::checklist::set_item_checked(&mut doc, &card_id, &checklist_id, &item_id, false)?;
                storage.save_board(&board_id, &mut doc)?;
                if json {
                    println!("{}", serde_json::json!({"checked": false, "item_id": item_id}));
                } else {
                    println!("Unchecked item {item_id}");
                }
            }
        },
        Commands::Space { cmd } => handle_space(cmd, &mut storage, &identity)?,
        Commands::Profile { cmd } => handle_profile(cmd, &mut storage, &identity, &dir)?,
        Commands::AiHelp => print_ai_help(),
        Commands::Sync { detach, stop, status, port, peers } => {
            cmd_sync(dir, detach, stop, status, port, peers).await?;
        }
    }
    Ok(())
}

async fn cmd_sync(
    data_dir: std::path::PathBuf,
    detach: bool,
    stop: bool,
    status: bool,
    port: u16,
    peers: Vec<String>,
) -> anyhow::Result<()> {
    use kanban_net::{NetworkHandle, NetConfig, NetEvent};
    use kanban_storage::Storage;
    use std::sync::{Arc, Mutex};

    let pid_file = data_dir.join("sync.pid");

    if stop {
        let pid_str = std::fs::read_to_string(&pid_file)
            .map_err(|_| anyhow::anyhow!("sync daemon not running (no PID file)"))?;
        let pid: i32 = pid_str.trim().parse()?;
        unsafe { libc::kill(pid, libc::SIGTERM); }
        println!("Stopped sync daemon (PID {pid})");
        return Ok(());
    }

    if status {
        match std::fs::read_to_string(&pid_file) {
            Ok(pid) => println!("Sync daemon running (PID {})", pid.trim()),
            Err(_) => println!("Sync daemon not running"),
        }
        return Ok(());
    }

    if detach {
        let exe = std::env::current_exe()?;
        let mut args: Vec<String> = std::env::args().collect();
        args.retain(|a| a != "--detach");
        let child = std::process::Command::new(exe)
            .args(&args[1..])
            .spawn()?;
        std::fs::write(&pid_file, child.id().to_string())?;
        println!("Sync daemon started (PID {})", child.id());
        return Ok(());
    }

    // Load identity bytes and space IDs
    let (identity_bytes, space_ids) = {
        let storage = Storage::open(&data_dir)?;
        let space_ids: Vec<String> = kanban_storage::space::list_spaces(storage.conn())?
            .into_iter().map(|s| s.id).collect();
        let key_path = data_dir.join("identity.key");
        let bytes = std::fs::read(&key_path)
            .map_err(|_| anyhow::anyhow!("Identity key not found. Run `monotask init` first."))?;
        let identity_bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid key file length"))?;
        (identity_bytes, space_ids)
    };

    let storage = Arc::new(Mutex::new(Storage::open(&data_dir)?));
    let mut handle = NetworkHandle::start(
        NetConfig { listen_port: port, data_dir: data_dir.clone(), bootstrap_peers: peers },
        Arc::clone(&storage),
        identity_bytes,
    ).await?;

    handle.announce_spaces(space_ids).await;

    // Snapshot current last_modified timestamps so we can detect CLI-side changes.
    let mut last_seen: std::collections::HashMap<String, i64> = {
        let guard = storage.lock().unwrap();
        let mut stmt = guard.conn()
            .prepare("SELECT board_id, last_modified FROM boards")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };

    // Use SyncTrigger so we can poll in the same select! that receives events.
    let sync_trigger = handle.sync_trigger();
    let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(2));

    println!("Sync daemon running. Press Ctrl+C to stop.");
    loop {
        tokio::select! {
            Some(event) = async { if let Some(rx) = handle.event_rx.as_mut() { rx.recv().await } else { None } } => {
                match event {
                    NetEvent::PeerConnected { peer_id } =>
                        println!("connected: {peer_id}"),
                    NetEvent::PeerDisconnected { peer_id } =>
                        println!("disconnected: {peer_id}"),
                    NetEvent::BoardSynced { board_id, peer_id } =>
                        println!("synced board {board_id} with {peer_id}"),
                    NetEvent::SyncError { board_id, error } =>
                        println!("sync error {board_id}: {error}"),
                }
            }
            _ = poll_interval.tick() => {
                // Detect boards modified by CLI commands (or any other process).
                let current: std::collections::HashMap<String, i64> = {
                    let guard = storage.lock().unwrap();
                    let mut stmt = guard.conn()
                        .prepare("SELECT board_id, last_modified FROM boards")
                        .unwrap_or_else(|_| panic!("failed to prepare board poll query"));
                    stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
                        .unwrap()
                        .filter_map(|r| r.ok())
                        .collect()
                };
                for (board_id, ts) in &current {
                    let old_ts = last_seen.get(board_id).copied().unwrap_or(0);
                    if *ts > old_ts {
                        println!("board {board_id:.8} changed (ts={ts}), triggering sync");
                        sync_trigger.trigger_sync(board_id.clone()).await;
                    }
                }
                last_seen = current;
            }
        }
    }
}

fn handle_space(cmd: SpaceCommands, storage: &mut kanban_storage::Storage, identity: &kanban_crypto::Identity) -> anyhow::Result<()> {
    use kanban_core::space as cs;
    use kanban_storage::space as ss;

    match cmd {
        SpaceCommands::Create { name } => {
            let space_id = uuid::Uuid::new_v4().to_string();
            let owner_pubkey = identity.public_key_hex();
            let mut doc = cs::create_space_doc(&name, &owner_pubkey)?;
            let profile = get_local_member_profile(storage.conn());
            cs::add_member(&mut doc, &owner_pubkey, &profile)?;
            let bytes = doc.save();
            ss::create_space(storage.conn(), &space_id, &name, &owner_pubkey, &bytes)?;
            let owner_member = cs::Member {
                pubkey: owner_pubkey.clone(),
                display_name: if profile.display_name.is_empty() { None } else { Some(profile.display_name.clone()) },
                avatar_blob: None,
                bio: None,
                role: None,
                color_accent: None,
                presence: None,
                kicked: false,
            };
            ss::upsert_member(storage.conn(), &space_id, &owner_member)?;
            println!("Created Space: {} ({})", name, space_id);
        }
        SpaceCommands::List => {
            let spaces = ss::list_spaces(storage.conn())?;
            if spaces.is_empty() {
                println!("No spaces found.");
            } else {
                for s in spaces {
                    println!("{} | {} | {} members", s.id, s.name, s.member_count);
                }
            }
        }
        SpaceCommands::Info { space_id } => {
            let space = ss::get_space(storage.conn(), &space_id)?;
            println!("Space: {} ({})", space.name, space.id);
            println!("Owner: {}", space.owner_pubkey);
            println!("Members ({}):", space.members.len());
            for m in &space.members {
                let name = m.display_name.as_deref().unwrap_or("(unnamed)");
                let kicked = if m.kicked { " [kicked]" } else { "" };
                println!("  {}  {}{}", &m.pubkey[..16], name, kicked);
            }
            println!("Boards ({}):", space.boards.len());
            for b in &space.boards {
                println!("  {}", b);
            }
        }
        SpaceCommands::Invite { cmd } => match cmd {
            SpaceInviteCommands::Generate { space_id } => {
                ss::revoke_all_invites(storage.conn(), &space_id)?;
                let doc_bytes = ss::load_space_doc(storage.conn(), &space_id)?;
                let token = kanban_crypto::generate_invite_token(&space_id, identity, Some(&doc_bytes))?;
                let meta = kanban_crypto::verify_invite_token_signature(&token)?;
                ss::insert_invite(storage.conn(), &meta.token_hash, &token, &space_id, None)?;
                println!("{}", token);
            }
            SpaceInviteCommands::Export { space_id, output_file } => {
                ss::revoke_all_invites(storage.conn(), &space_id)?;
                let doc_bytes = ss::load_space_doc(storage.conn(), &space_id)?;
                let token = kanban_crypto::generate_invite_token(&space_id, identity, Some(&doc_bytes))?;
                let meta = kanban_crypto::verify_invite_token_signature(&token)?;
                ss::insert_invite(storage.conn(), &meta.token_hash, &token, &space_id, None)?;
                let space = ss::get_space(storage.conn(), &space_id)?;
                use base64::Engine;
                let space_doc_b64 = base64::engine::general_purpose::STANDARD.encode(&doc_bytes);
                let payload = serde_json::json!({
                    "token": token,
                    "space_name": space.name,
                    "space_doc": space_doc_b64,
                });
                std::fs::write(&output_file, serde_json::to_string_pretty(&payload)?)?;
                println!("Exported invite to {}", output_file);
            }
            SpaceInviteCommands::Revoke { space_id } => {
                ss::revoke_all_invites(storage.conn(), &space_id)?;
                println!("Revoked all active invites for {}", space_id);
            }
        },
        SpaceCommands::Join { token_or_file } => {
            let local_pubkey = identity.public_key_hex();
            let (token, _hint_name, file_doc_opt) = parse_token_or_file(&token_or_file)?;
            let meta = kanban_crypto::verify_invite_token_signature(&token)?;
            ss::check_invite_policy(storage.conn(), &meta, &local_pubkey)?;

            // Prefer doc from token (v2), fall back to .space file payload, then stub
            let resolved_doc = meta.space_doc.clone().or(file_doc_opt);

            // If already a member but we now have a doc, update name + boards
            if let Ok(existing) = ss::get_space(storage.conn(), &meta.space_id) {
                if existing.members.iter().any(|m| m.pubkey == local_pubkey) {
                    if let Some(ref bytes) = resolved_doc {
                        let mut doc = automerge::AutoCommit::load(bytes)?;
                        let boards = cs::list_board_refs(&doc)?;
                        let members = cs::list_members(&doc)?;
                        let new_name = cs::get_space_name(&doc).unwrap_or(existing.name);
                        ss::update_space_doc(storage.conn(), &meta.space_id, &doc.save())?;
                        ss::rename_space(storage.conn(), &meta.space_id, &new_name)?;
                        for m in &members { let _ = ss::upsert_member(storage.conn(), &meta.space_id, m); }
                        for b in &boards { let _ = ss::add_board(storage.conn(), &meta.space_id, b); }
                        println!("Updated Space: {} ({})", new_name, meta.space_id);
                    } else {
                        println!("Already a member of Space: {} ({})", existing.name, meta.space_id);
                    }
                    return Ok(());
                }
            }
            let local_profile = get_local_member_profile(storage.conn());
            let (mut doc, members, boards, space_name) = if let Some(bytes) = resolved_doc {
                let doc = automerge::AutoCommit::load(&bytes)?;
                let name = cs::get_space_name(&doc).unwrap_or_else(|| "Shared Space".into());
                let members = cs::list_members(&doc)?;
                let boards = cs::list_board_refs(&doc)?;
                (doc, members, boards, name)
            } else {
                let mut doc = cs::create_space_doc("Shared Space", &meta.owner_pubkey)?;
                let empty = cs::MemberProfile { display_name: String::new(), avatar_b64: String::new(), bio: String::new(), role: String::new(), color_accent: String::new(), presence: String::new(), kicked: false };
                cs::add_member(&mut doc, &meta.owner_pubkey, &empty)?;
                let stub_owner = cs::Member {
                    pubkey: meta.owner_pubkey.clone(),
                    display_name: None,
                    avatar_blob: None,
                    bio: None,
                    role: None,
                    color_accent: None,
                    presence: None,
                    kicked: false,
                };
                (doc, vec![stub_owner], vec![], "Shared Space".into())
            };
            cs::add_member(&mut doc, &local_pubkey, &local_profile)?;
            let doc_bytes = doc.save();
            let _ = ss::create_space(storage.conn(), &meta.space_id, &space_name, &meta.owner_pubkey, &doc_bytes);
            for m in &members {
                let _ = ss::upsert_member(storage.conn(), &meta.space_id, m);
            }
            let local_sql = cs::Member {
                pubkey: local_pubkey,
                display_name: if local_profile.display_name.is_empty() { None } else { Some(local_profile.display_name) },
                avatar_blob: None,
                bio: None,
                role: None,
                color_accent: None,
                presence: None,
                kicked: false,
            };
            ss::upsert_member(storage.conn(), &meta.space_id, &local_sql)?;
            for b in &boards {
                let _ = ss::add_board(storage.conn(), &meta.space_id, b);
            }
            println!("Joined Space: {} ({})", space_name, meta.space_id);
        }
        SpaceCommands::Boards { cmd } => match cmd {
            SpaceBoardsCommands::Add { space_id, board_id } => {
                let bytes = ss::load_space_doc(storage.conn(), &space_id)?;
                let mut doc = automerge::AutoCommit::load(&bytes)?;
                cs::add_board_ref(&mut doc, &board_id)?;
                ss::update_space_doc(storage.conn(), &space_id, &doc.save())?;
                ss::add_board(storage.conn(), &space_id, &board_id)?;
                println!("Added board {} to Space {}", board_id, space_id);
            }
            SpaceBoardsCommands::Remove { space_id, board_id } => {
                let bytes = ss::load_space_doc(storage.conn(), &space_id)?;
                let mut doc = automerge::AutoCommit::load(&bytes)?;
                cs::remove_board_ref(&mut doc, &board_id)?;
                ss::update_space_doc(storage.conn(), &space_id, &doc.save())?;
                ss::remove_board(storage.conn(), &space_id, &board_id)?;
                println!("Removed board {} from Space {}", board_id, space_id);
            }
            SpaceBoardsCommands::List { space_id } => {
                let space = ss::get_space(storage.conn(), &space_id)?;
                for b in &space.boards { println!("{}", b); }
            }
        },
        SpaceCommands::Members { cmd } => match cmd {
            SpaceMembersCommands::List { space_id } => {
                let space = ss::get_space(storage.conn(), &space_id)?;
                for m in &space.members {
                    let name = m.display_name.as_deref().unwrap_or("(unnamed)");
                    let kicked = if m.kicked { " [kicked]" } else { "" };
                    println!("{}  {}{}", m.pubkey, name, kicked);
                }
            }
            SpaceMembersCommands::Kick { space_id, pubkey } => {
                let bytes = ss::load_space_doc(storage.conn(), &space_id)?;
                let mut doc = automerge::AutoCommit::load(&bytes)?;
                cs::kick_member(&mut doc, &pubkey)?;
                ss::update_space_doc(storage.conn(), &space_id, &doc.save())?;
                ss::set_member_kicked(storage.conn(), &space_id, &pubkey, true)?;
                println!("Kicked {} from Space {}", pubkey, space_id);
            }
        },
    }
    Ok(())
}

fn handle_profile(cmd: ProfileCommands, storage: &mut kanban_storage::Storage, identity: &kanban_crypto::Identity, data_dir: &std::path::Path) -> anyhow::Result<()> {
    use kanban_storage::space as ss;

    match cmd {
        ProfileCommands::Show => {
            let profile = ss::get_profile(storage.conn())?
                .unwrap_or_else(|| kanban_core::space::UserProfile {
                    pubkey: identity.public_key_hex(),
                    display_name: None,
                    avatar_blob: None,
                    bio: None,
                    role: None,
                    color_accent: None,
                    presence: None,
                    ssh_key_path: None,
                });
            println!("Pubkey:       {}", profile.pubkey);
            println!("Display name: {}", profile.display_name.as_deref().unwrap_or("(not set)"));
            println!("Avatar:       {}", if profile.avatar_blob.is_some() { "set" } else { "not set" });
            println!("SSH key path: {}", profile.ssh_key_path.as_deref().unwrap_or("(auto-generated)"));
        }
        ProfileCommands::SetName { name } => {
            let existing = ss::get_profile(storage.conn())?.unwrap_or_else(|| kanban_core::space::UserProfile {
                pubkey: identity.public_key_hex(),
                display_name: None,
                avatar_blob: None,
                bio: None,
                role: None,
                color_accent: None,
                presence: None,
                ssh_key_path: None,
            });
            ss::upsert_profile(storage.conn(), &kanban_core::space::UserProfile {
                display_name: Some(name.clone()),
                ..existing
            })?;
            println!("Display name set to: {}", name);
        }
        ProfileCommands::SetAvatar { path } => {
            let avatar_blob = std::fs::read(&path)?;
            let existing = ss::get_profile(storage.conn())?.unwrap_or_else(|| kanban_core::space::UserProfile {
                pubkey: identity.public_key_hex(),
                display_name: None,
                avatar_blob: None,
                bio: None,
                role: None,
                color_accent: None,
                presence: None,
                ssh_key_path: None,
            });
            ss::upsert_profile(storage.conn(), &kanban_core::space::UserProfile {
                avatar_blob: Some(avatar_blob),
                ..existing
            })?;
            println!("Avatar set from {}", path);
        }
        ProfileCommands::ImportSshKey { path } => {
            let path_ref = path.as_deref().map(std::path::Path::new);
            let new_identity = kanban_crypto::import_ssh_identity(path_ref)?;
            let pubkey = new_identity.public_key_hex();
            let key_bytes = new_identity.to_secret_bytes();
            std::fs::write(data_dir.join("identity.key"), key_bytes)?;
            let existing = ss::get_profile(storage.conn())?;
            ss::upsert_profile(storage.conn(), &kanban_core::space::UserProfile {
                pubkey: pubkey.clone(),
                display_name: existing.as_ref().and_then(|p| p.display_name.clone()),
                avatar_blob: existing.as_ref().and_then(|p| p.avatar_blob.clone()),
                bio: existing.as_ref().and_then(|p| p.bio.clone()),
                role: existing.as_ref().and_then(|p| p.role.clone()),
                color_accent: existing.as_ref().and_then(|p| p.color_accent.clone()),
                presence: existing.as_ref().and_then(|p| p.presence.clone()),
                ssh_key_path: path,
            })?;
            println!("Imported SSH key. New pubkey: {}", pubkey);
        }
    }
    Ok(())
}

fn get_local_member_profile(conn: &rusqlite::Connection) -> kanban_core::space::MemberProfile {
    use kanban_storage::space as ss;
    let profile = ss::get_profile(conn).ok().flatten();
    kanban_core::space::MemberProfile {
        display_name: profile.as_ref()
            .and_then(|p| p.display_name.clone())
            .unwrap_or_default(),
        avatar_b64: profile.as_ref()
            .and_then(|p| p.avatar_blob.as_ref())
            .map(|b| { use base64::Engine; base64::engine::general_purpose::STANDARD.encode(b) })
            .unwrap_or_default(),
        bio: "".into(),
        role: "".into(),
        color_accent: "".into(),
        presence: "".into(),
        kicked: false,
    }
}

fn print_ai_help() {
    print!("{}", r#"
================================================================================
MONOTASK CLI – AI AGENT REFERENCE
================================================================================
Binary : monotask
Version: 0.1.0
Purpose: P2P task manager with local-first CRDT storage. Designed for
         task management, collaborative workspaces, and automation via CLI.

Run `monotask ai-help` to print this document at any time.

--------------------------------------------------------------------------------
QUICK-START FOR AGENTS
--------------------------------------------------------------------------------
1. Check your identity:       monotask profile show
2. Create a board:            monotask board create "My Project"
3. List columns on a board:   monotask column list <BOARD_ID>
4. Create a card:             monotask card create <BOARD_ID> <COL_ID> "Task title"
5. View a card:               monotask card view <BOARD_ID> <CARD_ID>
6. Add a comment:             monotask card comment add <BOARD_ID> <CARD_ID> "text"

Always use --json for machine-readable output when parsing results.

--------------------------------------------------------------------------------
GLOBAL FLAGS
--------------------------------------------------------------------------------
--data-dir <PATH>
    Override the storage directory (default: $XDG_DATA_HOME/monotask or
    ~/.local/share/monotask on Linux/macOS).
    The directory contains:
      monotask.db  – SQLite database (boards, spaces, profile, invites)
      identity.key – Raw 32-byte Ed25519 secret key (auto-created on first run)

--------------------------------------------------------------------------------
IDENTITY & AUTHENTICATION
--------------------------------------------------------------------------------
Every user has an Ed25519 keypair. The public key (hex, 64 chars) is your
persistent identity across all operations.

Identity resolution order (first found wins):
  1. SSH Ed25519 key at path stored in profile (set via `profile import-ssh-key`)
  2. identity.key file in data directory
  3. Auto-generated key (written to identity.key on first run)

Your public key is used as:
  - Space ownership and membership
  - Card authorship (created_by field)
  - Invite token signing/verification

--------------------------------------------------------------------------------
COMMANDS
--------------------------------------------------------------------------------

## init
Usage: monotaskinit
Effect: Prints the data directory path. Triggers identity creation if missing.
        Safe to run multiple times (idempotent).

────────────────────────────────────────────────────────────────────────────────
## board
Boards are the top-level containers. Each board holds an ordered list of
columns; each column holds an ordered list of cards. Boards are stored as
Automerge CRDT documents (binary blobs in SQLite).

### board create <TITLE>
  --json   Output JSON

  Creates a new board with the given title. Prints the board ID.
  Text output:  "Created board: <title> (<id>)"
  JSON output:  {"id":"<uuid>","title":"<title>","deep_link":"monotask://board/<id>"}

  Example:
    $ monotask board create "Sprint 42" --json
    {"id":"a1b2c3d4-...","title":"Sprint 42","deep_link":"monotask://board/a1b2c3..."}

### board list
  --json   Output JSON

  Lists all board IDs stored locally.
  Text output:  one board ID per line
  JSON output:  ["<uuid>", ...]

  Example:
    $ monotaskboard list --json
    ["a1b2c3d4-...","e5f6a7b8-..."]

────────────────────────────────────────────────────────────────────────────────
## column
Columns are ordered within a board. Each column has an ID and a title and
maintains an ordered list of card IDs.

### column create <BOARD_ID> <TITLE>
  --json   Output JSON

  Creates a new column in the specified board.
  Text output:  "Created column: <title> (<id>)"
  JSON output:  {"id":"<uuid>","board_id":"<board_id>"}

  Example:
    $ monotaskcolumn create a1b2c3d4-... "In Progress" --json
    {"id":"c9d0e1f2-...","board_id":"a1b2c3d4-..."}

### column list <BOARD_ID>
  --json   Output JSON

  Lists all columns in the board in order.
  Text output:  "<col_id>: <title>"  (one per line)
  JSON output:  [{"id":"...","title":"...","card_ids":["..."]}, ...]

  Note: card_ids is the ordered list of card UUIDs in each column.

  Example:
    $ monotask column list a1b2c3d4-... --json
    [{"id":"c9d0e1f2-...","title":"Todo","card_ids":[]},
     {"id":"d3e4f5a6-...","title":"Done","card_ids":["card-uuid-..."]}]

────────────────────────────────────────────────────────────────────────────────
## card
Cards are the primary work items. Each card belongs to exactly one column.

Card fields:
  id          – UUID (use this for all card operations)
  number      – Human-readable short ID, format "<prefix>-<seq>" e.g. "a7f3-1"
                Prefix = first 4 chars of base32-encoded creator pubkey.
                Sequence = per-creator counter (1, 2, 3, ...).
  title       – Short summary string
  description – Long-form markdown text (may be empty)
  assignees   – List of pubkey strings (future: currently unused in CLI)
  labels      – List of label strings
  due_date    – Optional date string "YYYY-MM-DD" or null
  archived    – Boolean (soft-archive, hidden from normal views)
  deleted     – Boolean (soft-delete, hidden from all views)
  created_by  – Hex pubkey of creator
  created_at  – HLC timestamp (see TIMESTAMPS section)

### card create <BOARD_ID> <COL_ID> <TITLE>
  --json   Output JSON

  Creates a card in the specified column of the specified board.
  Text output:  "Created card: <title> (<id>)"
  JSON output:  {"id":"<uuid>","title":"<title>","board_id":"<board_id>","number":"<prefix>-<n>"}

  Example:
    $ monotask card create a1b2-... c9d0-... "Fix login bug" --json
    {"id":"f1a2b3c4-...","title":"Fix login bug","board_id":"a1b2-...","number":"aaaa-1"}

### card view <BOARD_ID> <CARD_ID>
  --json   Output JSON

  Reads and prints all fields of a single card.
  Text output:  labelled key-value lines (ID, Title, Description, Status, Due)
  JSON output:  full Card struct as JSON

  JSON schema:
    {
      "id": "<uuid>",
      "number": {"prefix":"<str>","seq":<int>} | null,
      "title": "<str>",
      "description": "<str>",
      "assignees": ["<pubkey>", ...],
      "labels": ["<str>", ...],
      "due_date": "<YYYY-MM-DD>" | null,
      "archived": false,
      "deleted": false,
      "copied_from": "<uuid>" | null,
      "created_by": "<hex-pubkey>",
      "created_at": "<hlc-timestamp>"
    }

  Example:
    $ monotask card view a1b2-... f1a2b3c4-... --json

### card comment add <BOARD_ID> <CARD_ID> <TEXT>
  --json   Output JSON

  Adds a comment to the card.
  JSON output:  {"id":"<uuid>","author":"<str>","text":"<str>","created_at":"<hlc>","deleted":false}

### card comment list <BOARD_ID> <CARD_ID>
  --json   Output JSON

  Lists all non-deleted comments on a card in chronological order.
  Text output:  "[<created_at>] <author>: <text>"
  JSON output:  array of comment objects

### card comment delete <BOARD_ID> <CARD_ID> <COMMENT_ID>
  --json   Output JSON

  Soft-deletes a comment (marked deleted=true, not returned in list).
  JSON output:  {"deleted":"<comment_id>"}

────────────────────────────────────────────────────────────────────────────────
## checklist
Checklists are ordered task lists attached to a card. A card can have multiple
checklists.

### checklist add <BOARD_ID> <CARD_ID> <TITLE>
  --json

  Creates a new checklist on the card.
  JSON output:  {"id":"<uuid>","title":"<str>","items":[]}

### checklist item-add <BOARD_ID> <CARD_ID> <CHECKLIST_ID> <TEXT>
  --json

  Adds an unchecked item to a checklist.
  JSON output:  {"id":"<uuid>","text":"<str>","checked":false}

### checklist item-check <BOARD_ID> <CARD_ID> <CHECKLIST_ID> <ITEM_ID>
  --json

  Marks a checklist item as checked.
  JSON output:  {"checked":true,"item_id":"<uuid>"}

### checklist item-uncheck <BOARD_ID> <CARD_ID> <CHECKLIST_ID> <ITEM_ID>
  --json

  Marks a checklist item as unchecked.
  JSON output:  {"checked":false,"item_id":"<uuid>"}

────────────────────────────────────────────────────────────────────────────────
## space
Spaces are shared containers that group boards and members. They enable
multi-user collaboration via signed invite tokens.

Space ownership: The creator is the owner (cannot be changed).
Members: Any user who joins via a valid invite token.
Boards: Boards are associated with a space; they can be on multiple spaces.

### space create <NAME>
  Creates a new space owned by the current user.
  Output:  "Created Space: <name> (<id>)"

### space list
  Lists all spaces stored locally.
  Output:  "<id> | <name> | <member_count> members"

### space info <SPACE_ID>
  Prints full details: name, owner pubkey, member list, board IDs.
  Members are shown as: "  <pubkey[0..16]>  <display_name>"

### space invite generate <SPACE_ID>
  Generates a new signed invite token (revokes previous tokens first).
  Output: the raw Base58 token string (share this with invitees)

  Token format:  Base58-encoded 120-byte payload
    Bytes 0-15:  space_id (raw UUID bytes)
    Bytes 16-47: owner Ed25519 pubkey (32 bytes)
    Bytes 48-55: creation timestamp (u64 big-endian unix ms)
    Bytes 56-119: Ed25519 signature over bytes 0-55

### space invite export <SPACE_ID> <OUTPUT_FILE>
  Generates an invite and writes a .space JSON file containing:
    {"token":"<base58>","space_name":"<str>","space_doc":"<base64-automerge>"}
  The .space file includes the full space CRDT document so the joiner gets
  the current member list and board references immediately.

### space invite revoke <SPACE_ID>
  Invalidates all active invite tokens for the space.
  Existing members are not affected; only new joins are blocked.

### space join <TOKEN_OR_FILE>
  Joins a space using either:
    - A raw Base58 token string
    - A path to a .space JSON file (recommended; includes space document)

  The command verifies the token signature, checks it hasn't been revoked,
  then adds the local user as a member of the space.
  Idempotent: safe to run again if already a member.
  Output: "Joined Space: <name> (<id>)"

### space boards add <SPACE_ID> <BOARD_ID>
  Associates a local board with the space.
  The board must already exist locally (created via `board create`).

### space boards remove <SPACE_ID> <BOARD_ID>
  Removes the board association from the space (board data is not deleted).

### space boards list <SPACE_ID>
  Prints one board ID per line for all boards in the space.

### space members list <SPACE_ID>
  Prints one member per line: "<pubkey>  <display_name>"
  Kicked members are shown with " [kicked]" suffix.

### space members kick <SPACE_ID> <PUBKEY>
  Marks a member as kicked in the space document and local DB.
  Kicked members cannot interact with the space (enforcement is app-level).

────────────────────────────────────────────────────────────────────────────────
## profile
Manages the local user's identity and display information.

### profile show
  Prints:
    Pubkey:       <64-char hex>
    Display name: <name> or "(not set)"
    Avatar:       "set" or "not set"
    SSH key path: <path> or "(auto-generated)"

### profile set-name <NAME>
  Sets your display name (shown to other space members).
  Example:  monotaskprofile set-name "Alice"

### profile set-avatar <PATH>
  Reads an image file (any format) and stores it as your avatar blob.
  Example:  monotaskprofile set-avatar ~/avatar.png

### profile import-ssh-key [PATH]
  Imports an OpenSSH Ed25519 private key as your identity.
  If PATH is omitted, defaults to ~/.ssh/id_ed25519
  The imported key replaces the current identity.key.
  WARNING: This changes your public key — space memberships tied to the old
           key will no longer match. Run this before joining any spaces.

--------------------------------------------------------------------------------
TIMESTAMPS (HLC FORMAT)
--------------------------------------------------------------------------------
All created_at / timestamp fields use Hybrid Logical Clock format:
  "<wall_ms_hex>-<logical_hex>"
  Example: "018f3a2b4c5d6e7f-00000001"
           wall_ms  = 018f3a2b4c5d6e7f (hex, Unix milliseconds)
           logical  = 00000001 (hex counter, increments on same-ms operations)

To convert to a Unix timestamp in milliseconds:
  ms = parseInt(hlc.split('-')[0], 16)
To convert to a human date (JavaScript):
  new Date(ms).toISOString()

--------------------------------------------------------------------------------
ID FORMATS
--------------------------------------------------------------------------------
Board ID   : UUID v4, e.g. "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
Column ID  : UUID v4
Card ID    : UUID v4  ← use this in all card/comment/checklist commands
Card Number: "<base32-prefix>-<seq>"  e.g. "a7f3-42"  (human-readable only;
             the CLI commands require the full UUID card ID, not the number)
Space ID   : UUID v4
Comment ID : UUID v4
Item ID    : UUID v4

--------------------------------------------------------------------------------
STORAGE
--------------------------------------------------------------------------------
Location:  ~/.local/share/monotask/monotask.db  (or custom --data-dir)

Database tables:
  boards            board_id | automerge_doc (BLOB) | last_modified | last_heads
  card_number_index board_id | card_id | number
  spaces            id | name | owner_pubkey | created_at | automerge_bytes
  space_members     space_id | pubkey | display_name | avatar_blob | kicked
  space_boards      space_id | board_id
  space_invites     token_hash (PK) | token | space_id | created_at | revoked
  user_profile      pk='local' | pubkey | display_name | avatar_blob | ssh_key_path

Board data is stored as Automerge CRDT binary documents. The root map contains:
  columns       – list of column objects [{id, title, card_ids[]}]
  cards         – map of card_id → card object
  members       – map of pubkey → member profile
  actor_card_seq – map of pubkey → int (per-actor card counter)
  label_definitions – map of label_id → label object

--------------------------------------------------------------------------------
COMMON AGENT WORKFLOWS
--------------------------------------------------------------------------------

### Workflow: Create a board and populate it
  BOARD=$(monotask board create "My Board" --json | jq -r .id)
  TODO_COL=$(monotaskcolumn create $BOARD "Todo" --json | jq -r .id)
  DOING_COL=$(monotaskcolumn create $BOARD "Doing" --json | jq -r .id)
  DONE_COL=$(monotaskcolumn create $BOARD "Done" --json | jq -r .id)
  CARD=$(monotask card create $BOARD $TODO_COL "First task" --json | jq -r .id)
  monotask card view $BOARD $CARD --json

### Workflow: Inspect all cards in a board
  # 1. List columns
  COLS=$(monotask column list $BOARD --json)
  # 2. For each column, iterate card_ids and call card view
  echo $COLS | jq -r '.[].card_ids[]' | while read CARD_ID; do
    monotask card view $BOARD $CARD_ID --json
  done

### Workflow: Collaborative space setup (two users, A and B)
  # --- User A ---
  SPACE=$(monotaskspace create "Team" | awk '{print $NF}' | tr -d '()')
  monotaskspace boards add $SPACE $BOARD
  monotaskspace invite export $SPACE invite.space
  # Share invite.space with User B

  # --- User B ---
  monotaskspace join invite.space
  monotaskspace boards list $SPACE   # see boards shared by A

### Workflow: Add a checklist to a card
  CL=$(monotaskchecklist add $BOARD $CARD "Definition of Done" --json | jq -r .id)
  ITEM=$(monotaskchecklist item-add $BOARD $CARD $CL "Write tests" --json | jq -r .id)
  monotaskchecklist item-check $BOARD $CARD $CL $ITEM

### Workflow: Comment thread
  monotask card comment add $BOARD $CARD "Starting work on this"
  monotask card comment add $BOARD $CARD "Blocked on API access"
  monotaskcard comment list $BOARD $CARD --json

--------------------------------------------------------------------------------
ERROR HANDLING
--------------------------------------------------------------------------------
All commands exit with code 0 on success, non-zero on error.
Errors are printed to stderr as plain text (not JSON).
Common error causes:
  - Board/card/column/space ID not found in local database
  - Invalid UUID format for IDs
  - Board file corrupted or missing
  - Invite token invalid signature or revoked
  - SSH key file not found or wrong format (must be Ed25519)

--------------------------------------------------------------------------------
LIMITATIONS & NOTES FOR AGENTS
--------------------------------------------------------------------------------
- The CLI does NOT sync between users automatically. P2P sync is handled
  by the desktop app (Monotask GUI). The CLI operates only on local data.
- `card create` currently uses a placeholder identity for card numbers
  (all cards get prefix "aaaa"). Full identity wiring is planned.
- There is no `card move` CLI command yet. Card column assignment is managed
  by the GUI. To get a card's current column: iterate column list and check
  which column's card_ids contains the card.
- `card view --json` returns the full Card struct; the `number` field is a
  JSON object {"prefix":"...","seq":N}, not the display string "prefix-N".
- Invite tokens are single-use per generation: generating a new token revokes
  the previous one. Use `invite export` (not `invite generate`) to share
  invites that include full space state.
- Data directory must be consistent across all CLI invocations for the same
  instance. If using --data-dir, always pass the same path.

================================================================================
"#);
}

fn parse_token_or_file(input: &str) -> anyhow::Result<(String, String, Option<Vec<u8>>)> {
    if input.ends_with(".space") || std::path::Path::new(input).exists() {
        let content = std::fs::read_to_string(input)?;
        let v: serde_json::Value = serde_json::from_str(&content)?;
        let token = v["token"].as_str()
            .ok_or_else(|| anyhow::anyhow!("missing or invalid 'token' field in .space file"))?
            .to_string();
        let name = v["space_name"].as_str().unwrap_or("Shared Space").to_string();
        let doc_b64 = v["space_doc"].as_str().unwrap_or("");
        let doc_bytes = if doc_b64.is_empty() {
            None
        } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(doc_b64).ok()
        };
        Ok((token, name, doc_bytes))
    } else {
        // Bare token — space_doc and name come from the embedded payload (v2 token)
        Ok((input.to_string(), String::new(), None))
    }
}
