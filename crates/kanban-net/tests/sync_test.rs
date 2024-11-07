use kanban_net::{NetworkHandle, NetConfig, NetEvent};
use kanban_storage::Storage;
use kanban_crypto::Identity;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn make_storage() -> Arc<Mutex<Storage>> {
    Arc::new(Mutex::new(Storage::open_in_memory().unwrap()))
}

fn make_identity() -> [u8; 32] {
    Identity::generate().to_secret_bytes()
}

/// Integration test: two NetworkHandle nodes start, mDNS discovers them on loopback,
/// and at least one side emits a PeerConnected event.
#[tokio::test]
async fn two_nodes_connect_and_emit_peer_connected() {
    let id_a = make_identity();
    let id_b = make_identity();
    let storage_a = make_storage();
    let storage_b = make_storage();

    let mut node_a = NetworkHandle::start(
        NetConfig { listen_port: 0, data_dir: std::path::PathBuf::from("/tmp/node_a") },
        storage_a,
        id_a,
    ).await.expect("node_a start");

    let node_b = NetworkHandle::start(
        NetConfig { listen_port: 0, data_dir: std::path::PathBuf::from("/tmp/node_b") },
        storage_b,
        id_b,
    ).await.expect("node_b start");

    // Give mDNS time to discover on loopback
    tokio::time::sleep(Duration::from_secs(3)).await;

    // One side should have received a PeerConnected event
    let timeout = Duration::from_secs(10);
    let found = tokio::time::timeout(timeout, async {
        loop {
            if let Ok(event) = node_a.event_rx.try_recv() {
                if matches!(event, NetEvent::PeerConnected { .. }) {
                    return true;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }).await;

    assert!(found.is_ok(), "nodes did not connect within 10 seconds");

    node_a.stop().await;
    node_b.stop().await;
}
