# P2P Chat Architecture

A complete technical reference for how Monotask implements serverless, encrypted, offline-capable peer-to-peer chat — written for engineers who want to port this design to a standalone chat application.

---

## Table of Contents

1. [Overview](#overview)
2. [Technology Stack](#technology-stack)
3. [Identity Layer](#identity-layer)
4. [P2P Networking Layer](#p2p-networking-layer)
5. [Peer Discovery](#peer-discovery)
6. [Membership & Authentication](#membership--authentication)
7. [Message Storage — CRDT Document Model](#message-storage--crdt-document-model)
8. [Sync Protocol](#sync-protocol)
9. [Chat Doc Lifecycle](#chat-doc-lifecycle)
10. [Message Data Model](#message-data-model)
11. [Inline References (@mentions, #refs)](#inline-references-mentions-refs)
12. [Identity in the UI](#identity-in-the-ui)
13. [Presence](#presence)
14. [Porting Guide — Building a Standalone Chat App](#porting-guide--building-a-standalone-chat-app)
15. [Known Trade-offs and Limitations](#known-trade-offs-and-limitations)

---

## Overview

Monotask's chat is **serverless and conflict-free**. Every peer stores the full message history locally. Peers sync directly with each other over encrypted connections. There is no central server, no message broker, no ordering authority. Two peers who were offline for a week will merge their diverged histories without data loss when they reconnect.

The key insight: **chat messages are just another CRDT document**, synced by the same pipeline that syncs kanban boards. No new protocol code was needed for chat.

```
Peer A writes message → appended to local automerge doc
Peer B connects       → automerge sync protocol exchanges delta
Peer B receives doc   → message appears instantly, in correct order
Peer C was offline    → catches up on reconnect, all messages intact
```

---

## Technology Stack

| Layer | Technology | Why |
|---|---|---|
| **P2P transport** | [libp2p](https://libp2p.io/) (Rust) | Handles NAT traversal, encryption, multiplexing, peer identity |
| **Transport protocol** | TCP + QUIC | TCP for reliability, QUIC for lower latency behind NAT |
| **Encryption** | Noise protocol (XX handshake) | Forward-secrecy session encryption over all connections |
| **Multiplexing** | Yamux | Multiple logical streams over one TCP connection |
| **Peer discovery (LAN)** | mDNS | Zero-config discovery on local networks |
| **Peer discovery (WAN)** | Kademlia DHT | Find peers on the internet via a distributed hash table |
| **NAT traversal** | libp2p relay + DCUtR | Hole-punching for peers behind routers |
| **Application protocol** | libp2p `request_response` | Custom CBOR-framed request/response over a named stream protocol |
| **CRDT / conflict-free storage** | [Automerge](https://automerge.org/) | Append-only list of messages; concurrent writes always merge |
| **Local persistence** | SQLite | Chat docs stored as blobs alongside board docs |
| **Identity** | Ed25519 keypair | Each user has a keypair; the public key is their permanent ID |
| **Message framing** | CBOR (via `ciborium`) | Compact binary serialization of all protocol messages |

---

## Identity Layer

Every participant has an **Ed25519 keypair** generated locally. The 32-byte public key (hex-encoded) is their permanent, unforgeable identity. It never changes, never requires registration.

```
user identity = Ed25519 keypair
  └─ private key  →  stays on device, used to sign space membership proofs
  └─ public key   →  shared openly, used as user ID everywhere
```

The same keypair drives the libp2p node identity. The 32-byte private key seed is passed directly into `libp2p::identity::ed25519::SecretKey::try_from_bytes`. This means:

- The libp2p `PeerId` is derived deterministically from the user's keypair
- Connection-level authentication (Noise) and application-level identity are the same key
- No separate "account" layer needed

```rust
// Bridge: app Ed25519 seed → libp2p keypair
let secret = libp2p::identity::ed25519::SecretKey::try_from_bytes(&mut key_bytes)?;
let ed_kp  = libp2p::identity::ed25519::Keypair::from(secret);
let keypair = identity::Keypair::from(ed_kp);
// keypair is now used for both Noise encryption AND PeerId derivation
```

**Profile fields** attached to an identity:
- `display_name` — human-readable name
- `avatar_b64` — base64 PNG, max 256×256
- `bio` — short status string
- `role` — job title / role label
- `color_accent` — hex color for UI chips
- `presence` — `online | away | dnd` (manual, not inferred from connection state)

Profiles are stored in the automerge **Space document** (a shared CRDT), so all peers in a space see each other's latest profile automatically on sync.

---

## P2P Networking Layer

The network layer is a single background Tokio task (`swarm::run`). The application communicates with it via two channels:

```
Application ──[NetCommand]──→ swarm task
Application ←──[NetEvent]──── swarm task
```

**NetCommand** (application → network):
```rust
AnnounceSpaces  { space_ids }    // tell DHT "I belong to these spaces"
TriggerSync     { board_id }     // sync this document with all connected peers
ForceRediscovery                 // re-announce + re-Hello all peers
AddPeer         { addr }         // manually dial a peer by multiaddr
GetPeers        { reply }        // ask for connected peer IDs
GetPeerPubkeys  { reply }        // ask for peer_id → pubkey_hex map
```

**NetEvent** (network → application):
```rust
PeerConnected    { peer_id }
PeerDisconnected { peer_id }
BoardSynced      { board_id, peer_id }
SyncError        { board_id, error }
```

**The swarm** is built with these libp2p behaviours composed together:

```rust
pub struct ComposedBehaviour {
    identify:      identify::Behaviour,      // exchange public keys + addresses on connect
    mdns:          mdns::tokio::Behaviour,   // LAN peer discovery
    kademlia:      kad::Behaviour<...>,      // WAN peer discovery via DHT
    relay_client:  relay::client::Behaviour, // connect through relay nodes
    dcutr:         dcutr::Behaviour,         // direct connection upgrade (hole-punch)
    autonat:       autonat::Behaviour,       // detect NAT type
    sync:          request_response::Behaviour<MonotaskCodec>, // our app protocol
}
```

---

## Peer Discovery

Two mechanisms run in parallel:

### LAN — mDNS

mDNS broadcasts a service announcement on the local network. Other Monotask nodes on the same subnet receive it and immediately dial back. Zero configuration, works offline, sub-second discovery.

### WAN — Kademlia DHT

The DHT key for a Space is:
```
SHA-256("monotask/space/{space_id}")
```

When you join a space, your node calls `kademlia.start_providing(key)` — advertising "I have data for this key." When you want to find peers in a space, you call `kademlia.get_providers(key)`.

```rust
pub fn space_dht_key(space_id: &str) -> RecordKey {
    let hash = Sha256::digest(format!("monotask/space/{space_id}").as_bytes());
    RecordKey::new(&hash)
}
```

The DHT bootstraps via public IPFS nodes (`bootstrap.libp2p.io`). This means discovery works across the internet with no dedicated server.

### NAT Traversal

For peers behind NAT:
1. **AutoNAT** detects whether the node is behind NAT
2. **relay client** connects to a public relay node as a fallback
3. **DCUtR** (Direct Connection Upgrade through Relay) attempts hole-punching to establish a direct connection, downgrading from relay to peer-to-peer

The connection timeout is set to 24 hours (`with_idle_connection_timeout(Duration::from_secs(24 * 3600))`) so long-lived connections between always-on nodes stay open.

---

## Membership & Authentication

Chat is **space-scoped**. Only members of a space can participate in its chat. Membership is enforced at the handshake layer.

### Joining a Space

Spaces use **signed invite tokens**. The space owner generates a token signed with their Ed25519 private key. The invite embeds the space document (automerge bytes) so the joiner immediately has the full member list and board refs.

### Hello Handshake

When two peers connect, one sends a `Hello` request:

```rust
SyncRequest::Hello {
    space_id:        String,
    board_ids:       Vec<String>,  // boards this peer has locally
    signature:       Vec<u8>,      // Ed25519 sig over space_id bytes
    space_doc_bytes: Vec<u8>,      // automerge-encoded space doc
}
```

The responder:
1. Checks the sender's pubkey (from the Identify protocol, established during connection)
2. Verifies the Ed25519 signature over `space_id`
3. Looks up the sender's pubkey in the local space member list
4. If not a member, or if kicked → responds `Rejected { reason }`
5. If valid → responds `HelloAck { space_id, board_ids, space_doc_bytes }`

After a successful Hello, both sides know which boards the other has. They start syncing.

**Pubkey extraction from PeerId:**
```rust
let hex_pubkey = pubkey_cache  // HashMap<PeerId, libp2p::identity::PublicKey>
    .get(&peer_id)
    .and_then(|pk| pk.clone().try_into_ed25519().ok())
    .map(|ed_pk| hex::encode(ed_pk.to_bytes()));
```
The `pubkey_cache` is populated by the Identify protocol on every new connection before the Hello is processed.

---

## Message Storage — CRDT Document Model

Each space has exactly one **chat document**: an automerge `AutoCommit` stored in the same SQLite `boards` table as kanban boards, with `is_system = 1` to hide it from the boards UI.

**Deterministic document ID:**
```
chat_doc_id = "{space_id}-chat"
```

Two peers who independently bootstrap the chat doc (before syncing) will create docs with the same ID. When automerge syncs them, concurrent inserts into the messages list both survive — no data loss.

### Why Automerge?

Automerge is a CRDT (Conflict-free Replicated Data Type). Key properties for chat:

- **Append-only list** — messages are inserted at the end; concurrent inserts from different peers both survive
- **No central ordering** — each peer appends locally; merge order is deterministic but not necessarily timestamp-ordered
- **Offline-first** — a peer can write messages while disconnected; they sync when reconnected
- **Delta sync** — only the changes since last sync are transmitted, not the full history
- **Convergent** — after syncing, all peers have identical state regardless of sync order

The tradeoff: automerge lists don't guarantee causal ordering. Two peers sending simultaneously may see their messages interleaved differently. Monotask sorts by `created_at` (unix timestamp) after reading, which works well in practice. For strict ordering in a chat app you'd need vector clocks or a sequencer.

---

## Sync Protocol

The wire protocol is `request_response` over a named libp2p stream:

```
Protocol: /monotask/board-sync/1.0.0
Framing:  4-byte big-endian length prefix + CBOR body
```

**Sync request variants:**
```rust
SyncRequest::Hello     { space_id, board_ids, signature, space_doc_bytes }
SyncRequest::BoardSync { board_id, sync_message: Vec<u8> }  // automerge::sync::Message
```

**Sync response variants:**
```rust
SyncResponse::HelloAck  { space_id, board_ids, space_doc_bytes }
SyncResponse::BoardSync { board_id, sync_message: Option<Vec<u8>> }  // None = converged
SyncResponse::Rejected  { reason: String }
```

### Automerge Sync State Machine

Automerge provides a built-in sync protocol (`automerge::sync`). Each peer maintains a `SyncState` per (peer, document) pair. The sync loop:

```
Peer A                             Peer B
  │  SyncRequest::BoardSync          │
  │  sync_message = A.generate(B)   │
  │ ──────────────────────────────→ │
  │                                  │  B.receive(msg)
  │                                  │  B.generate(A)
  │  SyncResponse::BoardSync         │
  │  sync_message = Some(B.gen)     │
  │ ←────────────────────────────── │
  │  A.receive(msg)                  │
  │  A.generate(B) → None           │
  │  SyncRequest::BoardSync          │
  │  sync_message = None (done)     │
  │ ──────────────────────────────→ │
```

`None` signals convergence. The protocol terminates when both sides have nothing new to send.

**Trigger points for chat sync** — `send_chat_message_cmd` calls `trigger_board_sync(chat_doc_id)` after every write. Peers also sync on:
- Initial Hello handshake (discover all shared docs)
- Reconnection after disconnect
- Periodic 4-second UI poll (which re-reads the local doc — no network sync, just reads what was already synced in background)

---

## Chat Doc Lifecycle

```
First peer opens chat panel for space_id
    └─ No local chat doc?
        └─ create_chat_doc() → empty automerge doc
        └─ save to boards table: id="{space_id}-chat", is_system=1
        └─ add_board_ref(space_doc, "{space_id}-chat")
        └─ space doc syncs to peers on next Hello
           └─ peers discover chat doc ID in space doc board refs
              └─ peers pull chat doc via BoardSync
```

The space document (also automerge) holds the list of board refs. When the chat doc ID is added to this list and the space doc syncs, all peers learn the chat doc exists and pull it.

If two peers independently bootstrap before syncing:
- Both create a doc with the same ID `{space_id}-chat`
- They each have an automerge doc with an empty messages list
- On sync: automerge merges them → still empty messages list (idempotent)
- Subsequent messages from both peers merge cleanly

---

## Message Data Model

### In-memory / wire (Rust structs)

```rust
pub struct ChatMessage {
    pub id:         String,    // UUID
    pub author:     String,    // pubkey hex (32 bytes = 64 hex chars)
    pub text:       String,    // raw message text
    pub created_at: u64,       // unix seconds
    pub refs:       Vec<ChatRef>,
}

pub struct ChatRef {
    pub kind:  String,  // "card" | "board" | "member"
    pub id:    String,  // entity UUID or pubkey
    pub label: String,  // display text at time of send
}
```

### In automerge (document structure)

```
ROOT
  messages: List<Map>
    [i]:
      id:         String
      author:     String   ← pubkey hex
      text:       String
      created_at: u64      ← stored as automerge Uint scalar
      refs:       List<Map>
        [j]:
          kind:   String
          id:     String
          label:  String
```

### Append a message

```rust
pub fn append_message(doc: &mut AutoCommit, msg: &ChatMessage) -> Result<()> {
    let (_, list_id) = doc.get(ROOT, "messages")?.unwrap();
    let len   = doc.length(&list_id);
    let entry = doc.insert_object(&list_id, len, ObjType::Map)?;
    doc.put(&entry, "id",         msg.id.as_str())?;
    doc.put(&entry, "author",     msg.author.as_str())?;
    doc.put(&entry, "text",       msg.text.as_str())?;
    doc.put(&entry, "created_at", msg.created_at)?;
    // refs sub-list ...
    Ok(())
}
```

### Read messages (newest-first, paginated)

```rust
pub fn list_messages(doc: &AutoCommit, limit: usize, before_ts: Option<u64>) -> Result<Vec<ChatMessage>> {
    // iterate list, apply before_ts filter, sort by created_at desc, truncate to limit
}
```

---

## Inline References (@mentions, #refs)

The `refs` field on each message carries structured references embedded in the text. When the user types `@Alice` or `#Fix login bug`, the autocomplete resolves it to a typed reference:

```json
{ "kind": "member", "id": "a3f2...pubkey...", "label": "Alice" }
{ "kind": "card",   "id": "card-uuid",        "label": "Fix login bug" }
{ "kind": "board",  "id": "board-uuid",        "label": "Sprint 1" }
```

The `label` is stored at send time (snapshot of the name). The `id` is permanent.

**Autocomplete backend:**
- **Members**: read from the in-memory space doc (instant, no DB query)
- **Boards**: `SELECT board_id, title FROM boards WHERE space_id = ? AND is_system = 0`
- **Cards**: separate `card_search_index` SQLite table (card_id, board_id, space_id, title, column_name), maintained at card CRUD. Queried as `WHERE space_id = ? AND title LIKE '%query%'`.

The card index avoids loading automerge board docs (which can be large) on every keypress.

---

## Identity in the UI

Peers are identified by pubkey hex, but displayed with rich profile information. The bridge between libp2p `PeerId` and profile data:

```
PeerId (from connection)
  → pubkey_cache: HashMap<PeerId, libp2p::identity::PublicKey>   [Identify protocol]
  → hex pubkey: try_into_ed25519().to_bytes() → hex::encode()
  → MemberProfile in space doc (automerge)
  → display_name, avatar_b64, color_accent, role, presence
```

This lookup happens in `get_sync_info_cmd`, which returns `Vec<PeerIdentityView>` to the frontend. The frontend renders identity chips (circular avatar or colored initial, name, role) instead of raw peer IDs.

**XSS protection**: all peer-controlled fields (display_name, role, bio, color_accent) are escaped before innerHTML insertion. CSS color values go through a strict validator:
```js
function safeCssColor(val) {
    return /^#[0-9a-fA-F]{3,8}$|^[a-zA-Z]+$|^rgba?\([^)]{1,40}\)$/.test(val)
        ? val : '#4a9a8a';
}
```

---

## Presence

Presence is **two separate signals**:

| Signal | Source | Meaning |
|---|---|---|
| **Presence field** | User-set in profile (`online / away / dnd`) | "How the user describes their availability" |
| **Live dot** | `connected_peers` from the swarm | "Currently connected right now" |

The presence field is stored in the space doc and synced like any other profile field. A peer can set their presence to "online" while being offline — it just means that was their last known status.

The live connection indicator is derived from `get_peers_sync()` which queries the swarm's active connections directly.

---

## Porting Guide — Building a Standalone Chat App

Here is the minimum viable set of components to extract from Monotask for a standalone P2P chat application.

### What to Keep (Core)

**1. Identity (`kanban-crypto`)**
- Ed25519 keypair generation + storage
- Signing / verification helpers
- Invite token generation (signed, single-use)

**2. Network layer (`kanban-net`)**
- `swarm.rs` — the full swarm with all behaviours
- `behaviour.rs` — composed behaviour definition
- `discovery.rs` — DHT key derivation, bootstrap peers
- `sync_protocol.rs` — the request/response codec

In a chat app, rename "Space" → "Room" or "Channel". The space concept maps directly.

**3. CRDT chat doc (`kanban-core/src/chat.rs`)**
- `create_chat_doc` / `append_message` / `list_messages`
- This is ~170 lines and completely standalone

**4. Storage**
- SQLite with two tables:
  - `rooms` — equivalent of `spaces` (room_id, name, owner_pubkey, automerge_bytes)
  - `docs` — equivalent of `boards` (doc_id, automerge_doc, last_modified, is_system)
  - `members` — room membership + profiles

### What to Drop

- Board/column/card types (`kanban-core/src/board.rs`, `kanban-core/src/card.rs`)
- The kanban UI
- `card_search_index` (replace with a message full-text index if needed)

### Simplified Data Flow for a Chat App

```
User types message
  → append_message(chat_doc, msg)    // local write
  → save_doc(room_id, chat_doc)      // persist to SQLite
  → net.trigger_sync(chat_doc_id)    // push to connected peers
  → UI polls get_messages every 3s   // pull from local doc
```

### The Room concept

A "room" in the ported app = a "space" in Monotask:

```rust
struct Room {
    id:          String,   // UUID, used to derive DHT key
    name:        String,
    owner_pubkey: String,  // only owner can kick members
    members:     Vec<MemberProfile>,
    doc_refs:    Vec<String>,  // list of chat doc IDs in this room
}
```

Stored as an automerge doc (so member list syncs automatically) plus a row in SQLite.

### Invite flow

```
Owner:   generate_invite(room_id)  →  Ed25519-signed token (base58)
Guest:   import_invite(token)      →  verifies sig, saves room doc locally
                                   →  announces room to DHT
Both:    discover each other via DHT, Hello handshake, start syncing
```

### Multi-room support

Each room gets its own chat doc (`{room_id}-chat`). A single swarm node manages all rooms. The Hello handshake carries the room_id so peers only exchange data for shared rooms.

### Scaling considerations

| Concern | Current approach | Production alternative |
|---|---|---|
| Message ordering | Client-side sort by unix timestamp | Vector clocks or HLC timestamps |
| History size | Full doc loaded in memory | Automerge compaction or separate index per time window |
| Large groups | DHT finds all peers, all sync directly | Relay node as hub for groups >20 peers |
| Message search | Client-side filter | SQLite FTS5 index, updated from automerge events |
| File attachments | Not implemented | IPFS content addressing + reference in ChatRef |
| Push notifications | Not implemented | WebPush or OS notification after sync |

### Cargo.toml dependencies for the extracted library

```toml
[dependencies]
# P2P
libp2p = { version = "0.53", features = [
    "tokio", "tcp", "quic", "noise", "yamux",
    "mdns", "kad", "identify", "relay", "dcutr", "autonat",
    "request-response", "macros"
] }
# CRDT
automerge = "0.5"
# Serialization
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"
ciborium     = "0.2"   # CBOR for wire protocol
# Crypto
ed25519-dalek = "2"
hex           = "0.4"
sha2          = "0.10"
# Async
tokio = { version = "1", features = ["full"] }
# Storage
rusqlite = { version = "0.31", features = ["bundled"] }
```

---

## Known Trade-offs and Limitations

| Issue | Impact | Notes |
|---|---|---|
| **No guaranteed message ordering** | Concurrent sends may interleave differently on each peer | Sort by `created_at`; use HLC for better ordering |
| **No read receipts** | Can't know if a message was delivered | Would require ACK messages in the CRDT or a side-channel |
| **No message deletion** | Automerge history is append-only | Deletions would need a separate "tombstone" list |
| **Full history always synced** | Old messages sync even if not needed | Implement time-windowed docs (one doc per day/week) |
| **No push while offline** | Messages only arrive when app is running | OS-level background sync or always-on relay node |
| **Presence is manual** | Online/away/dnd is user-set, not inferred | Track last-seen timestamp from connection events |
| **DHT bootstrap requires internet** | No WAN discovery if bootstrap nodes are down | Add your own bootstrap node; mDNS still works on LAN |
| **Single device per identity** | Keypair is on one machine | Multi-device would require key sync (out of scope) |

---

*Monotask source: `crates/kanban-core/src/chat.rs`, `crates/kanban-net/`, `crates/kanban-tauri/src-tauri/src/main.rs`*
