# P2P Sync Implementation Design

**Goal:** Implement automatic peer-to-peer board sync using libp2p so boards shared in a Space sync in real-time across local networks and the internet — no server, no account.

**Architecture:** libp2p Swarm with composed behaviours (Identify + mDNS + Kademlia + Relay + DCUtR + AutoNAT + custom sync protocol). Automerge's built-in sync state machine carries board changes over a custom `/monotask/board-sync/1.0.0` protocol. Ed25519 identity bridges directly to libp2p PeerId.

**Tech Stack:** `libp2p 0.53`, `automerge 0.5` (sync module), `tokio`, `ciborium` (already in kanban-net)

---

## 1. Identity & Transport

The existing Ed25519 keypair from `kanban-crypto` becomes the libp2p peer identity. The conversion:

```rust
// kanban-crypto::Identity exposes the 32 raw secret bytes
let secret_bytes = identity.to_secret_bytes(); // [u8; 32]
let lp2p_kp = libp2p::identity::ed25519::Keypair::try_from_bytes(&mut secret_bytes.clone())?;
let keypair = libp2p::identity::Keypair::from(lp2p_kp);
let peer_id = keypair.public().to_peer_id();
```

Note: the workspace `ed25519-dalek = "2"` and libp2p's internal dalek are separate crates — bridge via raw bytes only, never cast between the two types directly.

**Transports (in priority order):**
- QUIC-v1 — primary; faster handshake, better NAT behaviour
- TCP + Noise + Yamux — fallback for environments where UDP is blocked

Both transports are encrypted. The Swarm is created with these two transports combined via `libp2p::Transport::or_transport`.

---

## 2. Discovery & NAT Traversal

Five behaviours compose inside `ComposedBehaviour` (all required):

| Behaviour | Purpose |
|-----------|---------|
| `libp2p::identify::Behaviour` | Exchanges observed addresses and protocol support; required by Kademlia and DCUtR |
| `libp2p::mdns::tokio::Behaviour` | Local subnet peer discovery, zero config |
| `libp2p::kad::Behaviour` | Kademlia DHT for internet peer discovery |
| `libp2p::relay::client::Behaviour` | Circuit Relay v2 client; fallback tunnel via public relay nodes |
| `libp2p::dcutr::Behaviour` | Upgrades relayed connections to direct via hole-punching |
| `libp2p::autonat::Behaviour` | Detects NAT type; informs relay usage decisions |
| `libp2p::request_response::Behaviour` | Carries our custom `/monotask/board-sync/1.0.0` protocol |

**Identify** must be listed first in the composed behaviour derive macro — it is a dependency of Kademlia address propagation and DCUtR. Without it, Kademlia silently fails to propagate external addresses.

### Local network — mDNS
`libp2p-mdns` (tokio runtime) broadcasts peer presence on the local subnet. Peers on the same WiFi or LAN discover each other within seconds with no configuration. mDNS events emit `MdnsEvent::Discovered` which feeds peer addresses into the Kademlia routing table.

### Internet — Kademlia DHT
`libp2p-kad` connects to public IPFS bootstrap nodes at startup to join the global DHT.

Bootstrap nodes:
```
/dnsaddr/bootstrap.libp2p.io/p2p/QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN
/dnsaddr/bootstrap.libp2p.io/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb
```

**DHT key format for Space discovery:**
```rust
let key_str = format!("monotask/space/{}", space_id_hyphenated_uuid);
let key_bytes = sha2::Sha256::digest(key_str.as_bytes());
let record_key = libp2p::kad::RecordKey::new(&key_bytes);
```
- `space_id` is always the hyphenated UUID string (e.g. `"550e8400-e29b-41d4-a716-446655440000"`)
- Provider records expire after ~24 hours; re-announce every 20 hours via a `tokio::time::interval`
- On startup: call `kad.start_providing(key)` for every Space the local node is in
- Call `kad.get_providers(key)` when looking for peers in a Space

### NAT traversal
- `libp2p-autonat` detects whether the node is behind NAT
- `libp2p-relay` (Circuit Relay v2 client) falls back to public IPFS relay nodes when direct connection fails
- `libp2p-dcutr` attempts hole-punching after relay connection established to upgrade to direct

---

## 3. Sync Protocol

### Custom protocol: `/monotask/board-sync/1.0.0`

Implemented as `libp2p::request_response::Behaviour` with a CBOR codec (`ciborium`).

**Message types:**

```rust
enum SyncRequest {
    // Step 1: prove membership and share board list
    Hello {
        space_id: String,          // hyphenated UUID
        board_ids: Vec<String>,    // boards this peer has in this space
        signature: Vec<u8>,        // Ed25519 sig over space_id bytes
    },
    // Step 2: Automerge sync message for one board
    BoardSync {
        board_id: String,
        sync_message: Vec<u8>,    // automerge::sync::Message serialized
    },
}

enum SyncResponse {
    // Membership accepted; here are my board IDs
    HelloAck {
        board_ids: Vec<String>,
    },
    // Automerge sync message in reply (may be empty = converged)
    BoardSync {
        board_id: String,
        sync_message: Option<Vec<u8>>,  // None = this side has converged
    },
    // Rejected (not in same space, invalid signature, member kicked)
    Rejected { reason: String },
}
```

**Handshake & sync flow:**

1. On `PeerConnected`, initiating peer sends `Hello { space_id, board_ids, signature }` where `signature = Ed25519.sign(space_id.as_bytes())`
2. Receiving peer:
   - Gets the sender's Ed25519 pubkey from the **Identify behaviour cache** — `PeerId` in libp2p 0.53 is an opaque multihash with no `.public_key()` method. The correct source is `IdentifyInfo::public_key` emitted by the `identify::Behaviour` on `Event::Received`. Cache `PeerId → PublicKey` in a `HashMap` inside the swarm event loop; use this cache at `Hello` receive time
   - Extracts 32 raw pubkey bytes: `public_key.try_into_ed25519()?.to_bytes()` — these are the bytes `kanban-crypto::Identity::verify` expects
   - Verifies `Hello.signature` over `space_id.as_bytes()` using those 32 bytes
   - Checks Space membership: acquire `Arc<Mutex<Storage>>` lock, call `kanban_storage::space::get_space(guard.conn(), &space_id)`, verify sender pubkey (hex-encoded) is in `space_members` with `kicked = false`. Release lock immediately after
   - If any check fails → `Rejected`
3. Receiver responds `HelloAck { board_ids }` with its own board list for that Space
4. Sync is **bidirectional**: the initiator drives sync for boards in the **union** (not just intersection):
   - For boards both peers have: full Automerge sync in both directions
   - For boards only the responder has: initiator creates a new empty `AutoCommit` doc (`kanban_core::new_board_doc()`), runs sync to receive the full board, then saves it
   - For boards only the initiator has: initiator sends its full doc; responder saves the new board
   - For each board, the loop (run by the initiator):
     ```rust
     let mut doc = storage.lock().load_board(&board_id)
         .unwrap_or_else(|_| kanban_core::new_board_doc());
     let mut sync_state = automerge::sync::SyncState::new();
     loop {
         let msg = doc.generate_sync_message(&mut sync_state)?;
         send BoardSync { board_id, sync_message: msg.map(|m| m.encode()) };
         let reply = receive_response(); // SyncResponse::BoardSync
         match reply.sync_message {
             Some(bytes) => {
                 doc.receive_sync_message(&mut sync_state,
                     automerge::sync::Message::decode(&bytes)?)?;
                 let mut guard = storage.lock();
                 guard.save_board(&board_id, &mut doc)?;  // &mut guard, not &storage
             }
             None => break, // peer has converged
         }
         if doc.generate_sync_message(&mut sync_state)?.is_none() { break; }
     }
     ```
5. After all boards synced, connection stays open for future incremental syncs

**Key automerge types:** `AutoCommit` (not `Automerge`), `automerge::sync::SyncState`, `automerge::sync::Message`

**Conflict resolution:** Automerge CRDT merges automatically. Last-write-wins per scalar field; list operations are interleaved by insertion order. No user intervention.

**Sync trigger:**
- On `PeerConnected` (as above)
- When a board is saved locally: `trigger_sync(board_id)` is debounced 500ms using a `tokio::time::sleep` inside a `tokio::select!` with a cancellation token — if another save arrives within 500ms, the timer resets

---

## 4. kanban-net Architecture

### Files

```
crates/kanban-net/src/
├── lib.rs              — NetworkHandle public API + event types
├── behaviour.rs        — ComposedBehaviour (all 7 libp2p behaviours)
├── sync_protocol.rs    — /monotask/board-sync/1.0.0 codec + handler
├── discovery.rs        — mDNS + Kademlia peer discovery + DHT announce
└── swarm.rs            — Swarm construction and main event loop
```

### Public API

```rust
pub struct NetworkHandle {
    cmd_tx: mpsc::Sender<NetCommand>,
    pub event_rx: mpsc::Receiver<NetEvent>,
}

impl NetworkHandle {
    // storage must be Arc<Mutex<Storage>> because Storage holds rusqlite::Connection (!Send)
    pub async fn start(config: NetConfig, storage: Arc<Mutex<Storage>>) -> Result<Self>;
    pub async fn stop(&self);
    pub async fn announce_spaces(&self, space_ids: Vec<String>);
    pub async fn trigger_sync(&self, board_id: String);
}

pub enum NetEvent {
    PeerConnected  { peer_id: String },   // PeerId.to_base58()
    PeerDisconnected { peer_id: String },
    BoardSynced    { board_id: String, peer_id: String },
    SyncError      { board_id: String, error: String },
}

pub struct NetConfig {
    pub listen_port: u16,    // default 7272; saved to data_dir/net.port on bind
    pub data_dir: PathBuf,
}
```

**`Storage` threading:** `Storage` holds `rusqlite::Connection` which is `!Send`. Always pass as `Arc<Mutex<Storage>>`. The net task acquires the lock only for load/save operations; it never holds it across await points.

---

## 5. CLI & GUI Integration

### CLI

New `monotask sync` command:

```
monotask sync              # foreground daemon, prints NetEvents as they arrive
monotask sync --detach     # spawns background process; PID written to {data_dir}/sync.pid
monotask sync --stop       # reads {data_dir}/sync.pid, sends SIGTERM
monotask sync --status     # reads sync.pid; prints connected peers and last sync times
```

`PeerId` displayed as `PeerId.to_base58()` string consistently in all output.

### GUI (Tauri)

- `NetworkHandle::start()` called on app startup
- New Tauri command: `get_sync_status_cmd` → `Vec<{ peer_id: String, display_name: String, boards_synced: u32 }>`
- Sidebar shows sync indicator per Space: 🟢 synced, 🔄 syncing, ⚫ no peers
- When `BoardSynced` event fires for the currently open board, the GUI reloads the board automatically

### Scope boundary

Only boards added to a Space are synced. Boards not in any Space remain local-only.

---

## 6. Error Handling & Edge Cases

| Scenario | Handling |
|----------|----------|
| Peer goes offline mid-sync | Automerge sync is idempotent; reconnect resumes from last `SyncState` |
| Simultaneous edits to same card | Automerge CRDT merges automatically |
| Kicked member reconnects | `Hello` rejected — storage query checks `kicked = false` |
| Large boards | Automerge sync sends only the diff (incremental) |
| Port 7272 in use | Bind to port 0 (OS assigns); save actual port to `data_dir/net.port` |
| DHT provider record expiry | Re-announce every 20h via `tokio::time::interval` |

---

## 7. Dependencies (kanban-net/Cargo.toml)

**First, promote `sha2` in the root `Cargo.toml` workspace dependencies to key-value form** (it is currently a bare version string `sha2 = "0.10"` which does not support `{ workspace = true }` syntax):
```toml
# workspace Cargo.toml — change this line:
sha2 = { version = "0.10" }
```

Then `kanban-net/Cargo.toml`:
```toml
[dependencies]
libp2p = { version = "0.53", features = [
    "tcp", "quic", "noise", "yamux",
    "mdns", "kad", "relay", "dcutr", "autonat",
    "request-response", "identify",
    "tokio",
] }
kanban-core    = { path = "../kanban-core" }
kanban-crypto  = { path = "../kanban-crypto" }
kanban-storage = { path = "../kanban-storage" }
automerge   = { workspace = true }
sha2        = { workspace = true }
thiserror   = { workspace = true }
tracing     = { workspace = true }
tokio       = { workspace = true }
serde       = { workspace = true }
ciborium    = "0.2"
```

---

## 8. Out of Scope (this iteration)

- Syncing Space membership changes (member join/kick) — boards only
- Conflict UI — Automerge handles silently
- End-to-end encryption beyond transport-layer Noise encryption
- Windows/Linux relay node hosting guide
- Mobile support
