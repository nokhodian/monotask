pub mod behaviour;
pub mod discovery;
pub mod sync_protocol;
pub mod swarm;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::sync::mpsc;
use kanban_storage::Storage;

#[derive(Debug, Error)]
pub enum NetError {
    #[error("libp2p error: {0}")]
    Libp2p(String),
    #[error("storage error: {0}")]
    Storage(#[from] kanban_storage::StorageError),
    #[error("sync error: {0}")]
    Sync(String),
    #[error("handshake rejected: {0}")]
    Rejected(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Configuration for the network node.
#[derive(Clone, Debug)]
pub struct NetConfig {
    /// UDP/TCP port to listen on. 0 = OS-assigned. Default: 7272.
    pub listen_port: u16,
    /// Data directory — port is saved here as `net.port` after bind.
    pub data_dir: PathBuf,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            listen_port: 7272,
            data_dir: PathBuf::from("."),
        }
    }
}

/// Events emitted by the network layer to callers.
#[derive(Debug, Clone)]
pub enum NetEvent {
    PeerConnected    { peer_id: String },
    PeerDisconnected { peer_id: String },
    BoardSynced      { board_id: String, peer_id: String },
    SyncError        { board_id: String, error: String },
}

/// Commands sent from callers into the network task.
#[derive(Debug)]
pub(crate) enum NetCommand {
    AnnounceSpaces { space_ids: Vec<String> },
    TriggerSync    { board_id: String },
    Stop,
}

/// Handle to the background network task.
pub struct NetworkHandle {
    pub(crate) cmd_tx: mpsc::Sender<NetCommand>,
    /// Receive sync events. Poll this in your event loop.
    pub event_rx: mpsc::Receiver<NetEvent>,
}

impl NetworkHandle {
    /// Start the network node in a background tokio task.
    ///
    /// `storage` must be `Arc<Mutex<Storage>>` because `rusqlite::Connection`
    /// is `!Send`. The lock is acquired only for load/save, never held across
    /// await points.
    pub async fn start(
        config: NetConfig,
        storage: Arc<Mutex<Storage>>,
        identity_bytes: [u8; 32],
    ) -> Result<Self, NetError> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<NetCommand>(64);
        let (event_tx, event_rx) = mpsc::channel::<NetEvent>(256);

        tokio::spawn(swarm::run(config, storage, identity_bytes, cmd_rx, event_tx));

        Ok(Self { cmd_tx, event_rx })
    }

    /// Tell the node which Spaces this peer belongs to (re-call after join/leave).
    pub async fn announce_spaces(&self, space_ids: Vec<String>) {
        let _ = self.cmd_tx.send(NetCommand::AnnounceSpaces { space_ids }).await;
    }

    /// Trigger immediate sync for a board (debounced inside the swarm task).
    pub async fn trigger_sync(&self, board_id: String) {
        let _ = self.cmd_tx.send(NetCommand::TriggerSync { board_id }).await;
    }

    /// Gracefully stop the network task.
    pub async fn stop(&self) {
        let _ = self.cmd_tx.send(NetCommand::Stop).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn net_config_has_defaults() {
        let cfg = NetConfig::default();
        assert_eq!(cfg.listen_port, 7272);
    }

    #[test]
    fn net_event_is_debug() {
        let e = NetEvent::PeerConnected { peer_id: "abc".into() };
        assert!(format!("{e:?}").contains("PeerConnected"));
    }
}
