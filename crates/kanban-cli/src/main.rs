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
}

#[derive(Subcommand)]
enum BoardCommands {
    Create { title: String, #[arg(long)] json: bool },
    List { #[arg(long)] json: bool },
}

#[derive(Subcommand)]
enum ColumnCommands {
    Create { board_id: String, title: String, #[arg(long)] json: bool },
}

#[derive(Subcommand)]
enum CardCommands {
    Create { board_id: String, col_id: String, title: String, #[arg(long)] json: bool },
    View { board_id: String, card_id: String, #[arg(long)] json: bool },
}

fn data_dir(cli: &Cli) -> std::path::PathBuf {
    cli.data_dir.clone().unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("p2p-kanban")
    })
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let dir = data_dir(&cli);
    let mut storage = kanban_storage::Storage::open(&dir)?;

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
        },
        Commands::Card { cmd } => match cmd {
            CardCommands::Create { board_id, col_id, title, json } => {
                let mut doc = storage.load_board(&board_id)?;
                let card = kanban_core::card::create_card(&mut doc, &col_id, &title)?;
                storage.save_board(&board_id, &mut doc)?;
                if json {
                    println!("{}", serde_json::json!({"id": card.id, "title": card.title, "board_id": board_id}));
                } else {
                    println!("Created card: {} ({})", card.title, card.id);
                }
            }
            CardCommands::View { board_id, card_id, json } => {
                let doc = storage.load_board(&board_id)?;
                let card_obj = kanban_core::card::get_card_obj(&doc, &card_id)?;
                let title = kanban_core::get_string(&doc, &card_obj, "title")?.unwrap_or_default();
                if json { println!("{}", serde_json::json!({"id": card_id, "title": title})); }
                else { println!("{}: {}", card_id, title); }
            }
        },
    }
    Ok(())
}
