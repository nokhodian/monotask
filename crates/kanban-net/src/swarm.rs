use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use kanban_storage::Storage;
use crate::{NetCommand, NetConfig, NetEvent};

pub async fn run(
    _config: NetConfig,
    _storage: Arc<Mutex<Storage>>,
    _identity_bytes: [u8; 32],
    _cmd_rx: mpsc::Receiver<NetCommand>,
    _event_tx: mpsc::Sender<NetEvent>,
) {
    // stub
}
