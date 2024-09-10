# P2P Sync Implementation Design

**Goal:** Implement automatic peer-to-peer board sync using libp2p so boards shared in a Space sync in real-time across local networks and the internet — no server, no account.

**Architecture:** libp2p Swarm with composed behaviours (mDNS + Kademlia + Relay + DCUtR + AutoNAT + custom sync protocol). Automerge's built-in sync state machine carries board changes over a custom `/monotask/board-sync/1.0.0` protocol. Ed25519 identity bridges directly to libp2p PeerId.

**Tech Stack:** `libp2p 0.54`, `automerge 0.5` (sync module), `tokio`, `ciborium` (already in kanban-net)

---

## 1. Identity & Transport

The existing Ed25519 keypair from `kanban-crypto` is reused as the libp2p peer identity. The keypair is loaded at startup and converted to a `libp2p::identity::Keypair::Ed25519`, producing the node's `PeerId`. No new identity concept is introduced.

**Transports (in priority order):**
- QUIC-v1 — primary; faster handshake, better NAT behaviour
- TCP + Noise + Yamux — fallback for environments where UDP is blocked

Both transports are encrypted. The Swarm is created with these two transports combined via `libp2p::Transport::or_transport`.

---

## 2. Discovery & NAT Traversal

Three layers compose automatically inside the Swarm's `NetworkBehaviour`:

### Local network — mDNS
`libp2p-mdns` broadcasts peer presence on the local subnet. Peers on the same WiFi or LAN discover each other within seconds with no configuration.

### Internet — Kademlia DHT
`libp2p-kad` connects to public IPFS bootstrap nodes at startup to join the global DHT. For each Space the local node belongs to, it publishes a provider record under the key `sha256("monotask/space/" + space_id)`. When looking for peers in a Space, it queries the DHT for providers of that key. This requires no dedicated server — the IPFS DHT is free, public infrastructure.

Bootstrap nodes (from IPFS):
```
/dnsaddr/bootstrap.libp2p.io/p2p/QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN
/dnsaddr/bootstrap.libp2p.io/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb
```

### NAT traversal
- `libp2p-autonat` — detects whether the node is behind NAT and classifies reachability
- `libp2p-relay` (Circuit Relay v2) — falls back to public IPFS relay nodes when direct connection fails
- `libp2p-dcutr` — upgrades relayed connections to direct via hole-punching when possible

---

## 3. Sync Protocol

### Custom protocol: `/monotask/board-sync/1.0.0`

Implemented as a `libp2p::request_response::Behaviour` with a custom codec (CBOR via `ciborium`).

**Message types:**

```rust
enum SyncRequest {
    // Step 1: prove Space membership
    Hello { space_id: String, signature: Vec<u8> },
    // Step 2: send Automerge sync message for a board
    BoardSync { board_id: String, sync_message: Vec<u8> },
}

enum SyncResponse {
    // Membership accepted, here are my board IDs in this space
    HelloAck { board_ids: Vec<String> },
    // Automerge sync message in reply
    BoardSync { board_id: String, sync_message: Vec<u8> },
    // No boards to sync
    Done,
    // Rejected (wrong space, invalid signature)
    Rejected,
}
```

**Handshake & sync flow:**

1. On `PeerConnected`, initiating peer sends `Hello { space_id, signature }` where signature = `Ed25519.sign(space_id_bytes)` with the local keypair
2. Receiving peer verifies signature against the sender's `PeerId` pubkey and checks they share the Space. If not, responds `Rejected`
3. If accepted, both peers exchange `board_ids` for the Space
4. For each board both peers have: run Automerge sync — send `BoardSync` messages back and forth using `automerge::sync::State` until sync state returns `None` (convergence)
5. Each merged board is saved via `kanban-storage`

**Conflict resolution:** Automerge handles all conflicts automatically via CRDT merge. No user intervention needed.

**Trigger:** Sync runs on peer connect and whenever a board is saved locally (debounced 500ms to avoid flooding on rapid edits).

---

## 4. kanban-net Architecture

### Files

```
crates/kanban-net/src/
├── lib.rs              — NetworkHandle public API + event types
├── behaviour.rs        — ComposedBehaviour (all libp2p behaviours combined)
├── sync_protocol.rs    — /monotask/board-sync/1.0.0 codec + handler
├── discovery.rs        — mDNS + Kademlia peer discovery logic
└── swarm.rs            — Swarm construction and main event loop
```

### Public API

```rust
pub struct NetworkHandle {
    cmd_tx: mpsc::Sender<NetCommand>,
    event_rx: mpsc::Receiver<NetEvent>,
}

impl NetworkHandle {
    pub async fn start(config: NetConfig, storage: Arc<Storage>) -> Result<Self>;
    pub async fn stop(&self);
    pub async fn announce_spaces(&self, space_ids: Vec<String>);
    pub async fn trigger_sync(&self, board_id: String);
}

pub enum NetEvent {
    PeerConnected { peer_id: String },
    PeerDisconnected { peer_id: String },
    BoardSynced { board_id: String, peer_id: String },
    SyncError { board_id: String, error: String },
}

pub struct NetConfig {
    pub listen_port: u16,           // default 7272
    pub data_dir: PathBuf,
}
```

---

## 5. CLI & GUI Integration

### CLI

New `monotask sync` command in `kanban-cli`:

```
monotask sync              # runs sync daemon in foreground, prints events
monotask sync --detach     # spawns background process (writes PID to data dir)
monotask sync --stop       # kills background daemon
monotask sync --status     # shows connected peers and last sync times
```

### GUI (Tauri)

- `NetworkHandle::start()` called on app startup
- New Tauri commands: `get_sync_status_cmd` → list of `{ peer_id, display_name, boards_synced }`
- Sidebar shows a small sync indicator per Space: green dot (synced), spinner (syncing), grey (no peers)
- Board reloads automatically when a `BoardSynced` event arrives for the current board

### Scope boundary

Only boards added to a Space are synced. Boards not in any Space remain local-only forever.

---

## 6. Error Handling & Edge Cases

- **Peer goes offline mid-sync**: Automerge sync is idempotent; reconnect resumes where it left off
- **Both peers edit same card simultaneously**: Automerge CRDT merges automatically; last-write-wins per field
- **Space revoked member reconnects**: `Hello` rejected — kicked members have their `kicked: true` flag checked before accepting handshake
- **Large boards (slow sync)**: Automerge sync is incremental; only the diff is sent, not the full document
- **Port already in use**: Falls back to a random available port; port is saved to data dir for CLI `--status`

---

## 7. Dependencies to Add (kanban-net/Cargo.toml)

```toml
libp2p = { version = "0.54", features = [
    "tcp", "quic", "noise", "yamux",
    "mdns", "kad", "relay", "dcutr", "autonat",
    "request-response", "identify",
    "tokio",
] }
ciborium = "0.2"   # already present
sha2 = { workspace = true }
```

---

## 8. Out of Scope (this iteration)

- Syncing Space membership changes (member join/kick) — boards only for now
- Conflict UI (Automerge handles silently)
- End-to-end encryption of board content beyond transport encryption (future)
- Windows/Linux relay node hosting guide
