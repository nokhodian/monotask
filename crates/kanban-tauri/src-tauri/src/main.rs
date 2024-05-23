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

fn load_identity(
    data_dir: &std::path::Path,
    conn: &rusqlite::Connection,
) -> Result<kanban_crypto::Identity, Box<dyn std::error::Error>> {
    use kanban_crypto::Identity;
    use kanban_storage::space as space_store;

    let key_path = data_dir.join("identity.key");

    // Step 1: read user_profile row
    if let Some(profile) = space_store::get_profile(conn)? {
        // Step 2: SSH key path set?
        if let Some(ssh_path) = &profile.ssh_key_path {
            let p = std::path::Path::new(ssh_path);
            if p.exists() {
                if let Ok(id) = kanban_crypto::import_ssh_identity(Some(p)) {
                    return Ok(id);
                }
            }
        }
        // Step 3: load from identity.key
        if key_path.exists() {
            let bytes = std::fs::read(&key_path)?;
            if bytes.len() == 32 {
                let arr: [u8; 32] = bytes.try_into().unwrap();
                return Ok(Identity::from_secret_bytes(&arr));
            }
        }
    }

    // Step 4: generate new identity
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

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");

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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
