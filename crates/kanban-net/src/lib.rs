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
    /// Optional list of peer multiaddrs to dial at startup (bypasses mDNS).
    /// Format: `/ip4/1.2.3.4/tcp/7272` or `/ip4/1.2.3.4/udp/7272/quic-v1`.
    pub bootstrap_peers: Vec<String>,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            listen_port: 7272,
            data_dir: PathBuf::from("."),
            bootstrap_peers: Vec::new(),
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
    AnnounceSpaces     { space_ids: Vec<String> },
    TriggerSync        { board_id: String },
    ForceRediscovery,
    AddPeer            { addr: String },
    GetPeers           { reply: tokio::sync::oneshot::Sender<Vec<String>> },
    GetListenAddrs     { reply: tokio::sync::oneshot::Sender<Vec<String>> },
    GetPeerPubkeys     { reply: tokio::sync::oneshot::Sender<std::collections::HashMap<String, String>> },
    Stop,
}

/// A cloneable handle that can send sync-trigger commands independently of the event receiver.
/// Useful when you need to drive both receiving events and triggering sync in the same loop.
#[derive(Clone)]
pub struct SyncTrigger(mpsc::Sender<NetCommand>);

impl SyncTrigger {
    /// Async version — call from an async context.
    pub async fn trigger_sync(&self, board_id: String) {
        let _ = self.0.send(NetCommand::TriggerSync { board_id }).await;
    }
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

    /// Trigger immediate sync for a board — safe to call from non-async Tauri commands.
    pub fn trigger_sync_sync(&self, board_id: String) {
        let _ = self.cmd_tx.try_send(NetCommand::TriggerSync { board_id });
    }

    /// Return a cloneable sender so you can trigger sync while also receiving events.
    pub fn sync_trigger(&self) -> SyncTrigger {
        SyncTrigger(self.cmd_tx.clone())
    }

    /// Synchronous version of announce_spaces — safe to call from non-async Tauri commands.
    pub fn announce_spaces_sync(&self, space_ids: Vec<String>) {
        let _ = self.cmd_tx.blocking_send(NetCommand::AnnounceSpaces { space_ids });
    }

    /// Re-announce spaces on DHT + re-Hello all connected peers. Call to force an immediate sync attempt.
    pub fn force_rediscovery_sync(&self) {
        let _ = self.cmd_tx.blocking_send(NetCommand::ForceRediscovery);
    }

    /// Dial a peer by multiaddr (e.g. `/ip4/1.2.3.4/tcp/7272`).
    pub fn add_peer_sync(&self, addr: String) {
        let _ = self.cmd_tx.blocking_send(NetCommand::AddPeer { addr });
    }

    /// Return currently connected peer IDs (synchronous, blocks briefly).
    pub fn get_peers_sync(&self) -> Vec<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.cmd_tx.blocking_send(NetCommand::GetPeers { reply: tx });
        rx.blocking_recv().unwrap_or_default()
    }

    /// Return a map of connected peer IDs → ed25519 hex pubkeys.
    /// Built from the swarm's Identify protocol cache.
    pub fn get_peer_pubkeys_sync(&self) -> std::collections::HashMap<String, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.cmd_tx.blocking_send(NetCommand::GetPeerPubkeys { reply: tx });
        rx.blocking_recv().unwrap_or_default()
    }

    /// Return the swarm's current listen addresses (synchronous, blocks briefly).
    pub fn get_listen_addrs_sync(&self) -> Vec<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.cmd_tx.blocking_send(NetCommand::GetListenAddrs { reply: tx });
        rx.blocking_recv().unwrap_or_default()
    }

    /// Compute the local libp2p PeerId from the identity seed.
    pub fn peer_id_from_identity(identity_bytes: [u8; 32]) -> String {
        let mut key_bytes = identity_bytes;
        let Ok(secret) = libp2p::identity::ed25519::SecretKey::try_from_bytes(&mut key_bytes) else {
            return String::new();
        };
        let ed_kp = libp2p::identity::ed25519::Keypair::from(secret);
        let keypair = libp2p::identity::Keypair::from(ed_kp);
        libp2p::PeerId::from_public_key(&keypair.public()).to_string()
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

    #[test]
    fn net_command_has_get_peer_pubkeys_variant() {
        // Compile-time check that the variant exists and has the right shape
        let (tx, _rx) = tokio::sync::oneshot::channel::<std::collections::HashMap<String, String>>();
        let _cmd = NetCommand::GetPeerPubkeys { reply: tx };
    }
}
