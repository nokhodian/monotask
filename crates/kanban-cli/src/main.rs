use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "app-cli", about = "P2P Kanban CLI")]
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
    Ok(base.join("p2p-kanban"))
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
        ssh_key_path: None,
    };
    space_store::upsert_profile(conn, &new_profile)?;
    Ok(id)
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let dir = data_dir(&cli)?;
    let mut storage = kanban_storage::Storage::open(&dir)?;
    let identity = load_cli_identity(&dir, storage.conn())?;

    match cli.command {
        Commands::Init => {
            println!("Initialized p2p-kanban at {}", dir.display());
        }
        Commands::Board { cmd } => match cmd {
            BoardCommands::Create { title, json } => {
                let id = kanban_crypto::Identity::generate();
                let (mut doc, board) = kanban_core::board::create_board(&title, &id.public_key_hex())?;
                storage.save_board(&board.id, &mut doc)?;
                if json {
                    let deep_link = format!("kanban://board/{}", board.id);
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
    }
    Ok(())
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
                let token = kanban_crypto::generate_invite_token(&space_id, identity)?;
                let meta = kanban_crypto::verify_invite_token_signature(&token)?;
                ss::insert_invite(storage.conn(), &meta.token_hash, &token, &space_id, None)?;
                println!("{}", token);
            }
            SpaceInviteCommands::Export { space_id, output_file } => {
                ss::revoke_all_invites(storage.conn(), &space_id)?;
                let token = kanban_crypto::generate_invite_token(&space_id, identity)?;
                let meta = kanban_crypto::verify_invite_token_signature(&token)?;
                ss::insert_invite(storage.conn(), &meta.token_hash, &token, &space_id, None)?;
                let space = ss::get_space(storage.conn(), &space_id)?;
                let doc_bytes = ss::load_space_doc(storage.conn(), &space_id)?;
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
            let (token, space_name, doc_bytes_opt) = parse_token_or_file(&token_or_file)?;
            let meta = kanban_crypto::verify_invite_token_signature(&token)?;
            ss::check_invite_policy(storage.conn(), &meta, &local_pubkey)?;
            // Idempotency
            if let Ok(existing) = ss::get_space(storage.conn(), &meta.space_id) {
                if existing.members.iter().any(|m| m.pubkey == local_pubkey) {
                    println!("Already a member of Space: {} ({})", existing.name, meta.space_id);
                    return Ok(());
                }
            }
            let local_profile = get_local_member_profile(storage.conn());
            let (mut doc, members, boards) = if let Some(bytes) = doc_bytes_opt {
                let doc = automerge::AutoCommit::load(&bytes)?;
                let members = cs::list_members(&doc)?;
                let boards = cs::list_board_refs(&doc)?;
                (doc, members, boards)
            } else {
                let mut doc = cs::create_space_doc(&space_name, &meta.owner_pubkey)?;
                let empty = cs::MemberProfile { display_name: String::new(), avatar_b64: String::new(), kicked: false };
                cs::add_member(&mut doc, &meta.owner_pubkey, &empty)?;
                let stub_owner = cs::Member {
                    pubkey: meta.owner_pubkey.clone(),
                    display_name: None,
                    avatar_blob: None,
                    kicked: false,
                };
                (doc, vec![stub_owner], vec![])
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
                avatar_blob: existing.and_then(|p| p.avatar_blob),
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
        kicked: false,
    }
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
        Ok((input.to_string(), "Shared Space".to_string(), None))
    }
}
