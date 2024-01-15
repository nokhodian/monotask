# Decentralized P2P Kanban System — Holistic Execution Plan v2.0

## Executive Summary

This plan specifies a fully decentralized, peer-to-peer Kanban board (comparable to Trello) built with a Rust core and a Tauri v2 desktop GUI. Data lives entirely on users' machines — no central server, no cloud dependency. Peers discover each other, exchange cryptographically signed events, and converge to a shared board state via CRDTs. The plan has been through two rounds of analysis: an exhaustive feature expansion (Phase 1) and a critical stress test (Phase 2) that caught and fixed several architectural mistakes present in the original spec.

**Key corrections from the original spec:**
- The "GUI shells out to CLI via `std::process::Command`" architecture is replaced with a shared Rust library crate — the original approach is a documented anti-pattern in Tauri v2.
- `sled` is removed as a storage recommendation — it is alpha-quality and not forward-compatible.
- Vector clocks are replaced with Hybrid Logical Clocks (HLC) — better performance characteristics for this use case.
- The conflict resolution strategy is upgraded from ad-hoc LWW/fractional-indexing to Automerge-based CRDTs with proper semantics.

---

## Core Objectives & Success Criteria

- **Fully decentralized**: Zero server infrastructure required. Two users on a LAN can use the system with no internet. Success metric: system operates with 0 external dependencies after install.
- **Real-time collaboration**: Board updates propagate to connected peers within 500ms on LAN, 2s on WAN. Success metric: measured latency in integration tests.
- **Conflict-free convergence**: Concurrent edits by disconnected peers converge to identical state upon reconnect. Success metric: property-based tests with simulated network partitions produce identical materialized state on all nodes.
- **Cryptographic integrity**: Every event is signed and verified. Tampered events are rejected. Success metric: injection of forged events is caught and rejected in adversarial tests.
- **Usable by non-technical users**: The Tauri GUI provides a drag-and-drop Kanban experience comparable to Trello. Success metric: user can create a board, add cards, and invite a peer within 2 minutes of first launch.
- **Offline-first**: Full functionality when disconnected; automatic sync when peers reconnect. Success metric: create 50 cards offline, reconnect, and verify all cards appear on peer within 10s.

---

## Scope

### In Scope
- Desktop application (macOS, Linux, Windows) via Tauri v2
- Rust CLI for headless/scriptable usage
- P2P networking with LAN discovery and WAN hole-punching
- CRDT-based conflict resolution for all board operations
- Cryptographic identity and event signing (Ed25519)
- Event sourcing with append-only log, snapshots, and pruning
- Board sharing via invite tokens
- Import/export for air-gapped sync
- Audit trail / activity history

### Out of Scope (explicitly)
- Mobile apps (iOS / Android) — future phase
- Web browser version — future phase (would require WebRTC transport layer)
- Real-time text co-editing within card descriptions (e.g., Google Docs-style cursors) — card descriptions use whole-field CRDT merging, not character-level
- File attachments on cards — future phase (requires content-addressed blob storage)
- Video/voice chat between peers
- Central relay infrastructure operated by the project (users can self-host relays)
- User permissions / roles beyond board membership (no "admin" vs "viewer" in MVP)

---

## Technical Stack

| Layer | Choice | Rationale |
|---|---|---|
| **Language** | Rust (2021 edition, MSRV 1.75+) | Required for Tauri, excellent for crypto/networking, no GC pauses |
| **P2P Networking** | **Iroh 0.96+** | Built-in QUIC, hole-punching (QNT standard), gossip protocol (`iroh-gossip`), relay fallback. Near-100% connection success rate vs libp2p's ~70%. Production-proven on 100K+ devices |
| **CRDT Engine** | **Automerge 2.0** (via `automerge-rs`) | Mature, Rust-native, built-in sync protocol, handles maps/lists/text. Avoids building custom CRDT from scratch |
| **Storage** | **SQLite** (via `rusqlite`) | Proven, zero-config, excellent tooling, handles append-only logs well. Fjall is promising but SQLite's maturity wins for a project this complex |
| **Serialization** | **CBOR** (via `ciborium`) | Schema-evolution-friendly, compact binary, self-describing. Used for event payloads |
| **Wire format** | **Automerge sync protocol** | Built into Automerge — handles efficient delta exchange between peers |
| **Cryptography** | **ed25519-dalek** | Pure Rust, constant-time, actively maintained, keys zeroed on drop |
| **GUI Framework** | **Tauri v2.4+** | Stable, audited, 10-20MB bundle vs Electron's 100MB+. Native Rust backend |
| **Frontend** | **SolidJS** + Tailwind CSS | Fine-grained reactivity (no virtual DOM diffing), excellent for real-time updates from Tauri events. Smaller runtime than React |
| **Drag-and-drop** | **@thisbeyond/solid-dnd** or custom Solid DnD | SolidJS-native DnD library |
| **CLI output** | **clap** v4 + **tabled** + **colored** | Standard Rust CLI stack with `--json` flag support |

### Crate Workspace Structure

```
p2p-kanban/
├── Cargo.toml                  # Workspace root
├── crates/
│   ├── kanban-core/            # Domain logic, CRDT operations, event types
│   │   ├── src/lib.rs
│   │   └── Cargo.toml
│   ├── kanban-crypto/          # Ed25519 keypair, signing, verification
│   │   ├── src/lib.rs
│   │   └── Cargo.toml
│   ├── kanban-net/             # Iroh networking, gossip, sync
│   │   ├── src/lib.rs
│   │   └── Cargo.toml
│   ├── kanban-storage/         # SQLite backend, snapshots, pruning
│   │   ├── src/lib.rs
│   │   └── Cargo.toml
│   ├── kanban-cli/             # CLI binary (clap commands)
│   │   ├── src/main.rs
│   │   └── Cargo.toml
│   └── kanban-tauri/           # Tauri app (calls kanban-core directly)
│       ├── src-tauri/
│       │   ├── src/main.rs     # Tauri commands wrapping kanban-core
│       │   └── Cargo.toml
│       └── src/                # SolidJS frontend
│           ├── App.tsx
│           └── ...
└── tests/
    ├── integration/            # Multi-node simulation tests
    └── property/               # Property-based CRDT convergence tests
```

**Critical architecture decision:** Both the CLI and the Tauri app import `kanban-core`, `kanban-crypto`, `kanban-net`, and `kanban-storage` as library crates. The Tauri app does NOT shell out to the CLI binary. This eliminates process-spawning overhead, gives type-safe IPC via Tauri commands, and avoids the fragile JSON-stdout parsing layer described in the original spec.

The CLI binary is a thin wrapper around the same library crates, used for headless servers, scripting, and debugging.

---

## Architecture Overview

### Component Map

```
┌─────────────────────────────────────────────────────┐
│                   Tauri v2 App                       │
│  ┌───────────────────────────────────────────────┐  │
│  │          SolidJS Frontend (WebView)            │  │
│  │  ┌─────────┐ ┌──────────┐ ┌───────────────┐  │  │
│  │  │ Board   │ │ Kanban   │ │ Activity      │  │  │
│  │  │ Dashboard│ │ View     │ │ Sidebar       │  │  │
│  │  │         │ │ (DnD)    │ │               │  │  │
│  │  └─────────┘ └──────────┘ └───────────────┘  │  │
│  └──────────────────┬────────────────────────────┘  │
│                     │ Tauri Commands + Channels      │
│  ┌──────────────────┴────────────────────────────┐  │
│  │           Tauri Rust Backend                   │  │
│  │  ┌────────────┐ ┌────────────┐ ┌───────────┐ │  │
│  │  │ kanban-core│ │ kanban-net │ │ kanban-   │ │  │
│  │  │ (CRDT ops) │ │ (Iroh P2P) │ │ storage   │ │  │
│  │  └────────────┘ └────────────┘ └───────────┘ │  │
│  │  ┌────────────┐                               │  │
│  │  │ kanban-    │                               │  │
│  │  │ crypto     │                               │  │
│  │  └────────────┘                               │  │
│  └───────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────┘

         ▲ Iroh QUIC         ▲ Iroh QUIC
         │                   │
    ┌────┴─────┐       ┌────┴─────┐
    │  Peer B  │       │  Peer C  │
    │ (Desktop │       │ (CLI     │
    │  or CLI) │       │  only)   │
    └──────────┘       └──────────┘
```

### Data Flow

1. **User action** (e.g., moves card) → SolidJS calls `invoke('move_card', {...})` → Tauri command handler
2. **Tauri backend** → `kanban-core` generates CRDT operation → `kanban-crypto` signs it → `kanban-storage` persists to SQLite → `kanban-net` broadcasts via Iroh gossip
3. **Incoming peer event** → `kanban-net` receives via Iroh → `kanban-crypto` verifies signature → `kanban-core` applies CRDT merge → `kanban-storage` persists → Tauri Channel pushes update to frontend
4. **Frontend** receives update via Channel listener → SolidJS reactive store updates → UI re-renders affected components

### Key Design Patterns

- **Event Sourcing + CRDT hybrid**: Automerge document IS the event log. Each change is an Automerge operation. The document can be serialized/deserialized as a single blob (snapshot) or synced incrementally (Automerge sync protocol).
- **Actor model for networking**: The Iroh networking layer runs as a Tokio task. Communication with the main thread via `tokio::sync::mpsc` channels.
- **Reactive state propagation**: Tauri Channels stream CRDT state changes to the frontend. SolidJS fine-grained reactivity ensures only affected DOM nodes update.

---

## Module-by-Module Feature Inventory

### Module 1: Cryptography & Identity (`kanban-crypto`)

#### MVP Features
| Feature | Description | Design Decision |
|---|---|---|
| **Keypair generation** | Ed25519 keypair generated on first `init`. Private key stored in OS keychain (via `keyring` crate) with fallback to encrypted file (`~/.config/p2p-kanban/identity.key`) | Use `ed25519-dalek` for key operations. **Do NOT store private key as plaintext file** — original spec was silent on this |
| **Node identity** | Public key = Node ID = User ID. Encoded as base32 for human-readability (e.g., `pk_7xq3m...`) | Base32 with `pk_` prefix for unambiguous identification |
| **Event signing** | Every Automerge change is wrapped in a signed envelope: `{ author: PubKey, change: bytes, signature: Signature, hlc: HybridTimestamp }` | Sign the CBOR-serialized change bytes, not a hash of them (avoids second-preimage issues) |
| **Signature verification** | All incoming changes verified before Automerge merge. Invalid signatures → drop event + log warning + optionally ban peer | Verification is on the hot path — ed25519-dalek batch verification for bulk sync |
| **User profiles** | Local alias/avatar stored as a special Automerge map entry keyed by public key. Each user can only edit their own profile entry | Profile is part of the board document. Conflict-free because each user writes to their own key |

#### Missing from Original Spec (Added)
| Feature | Why It Matters |
|---|---|
| **Key backup/recovery** | If a user loses their private key, they lose their identity. Must support encrypted key export (`app-cli identity export --passphrase`) and import |
| **Key rotation** | Users need to be able to rotate compromised keys. Requires a "key succession" event signed by the old key that delegates trust to a new key |
| **Private key protection** | OS keychain integration (`keyring` crate) with encrypted-file fallback. Never store plaintext keys |
| **Peer banning** | Board creators should be able to publish a "ban" event for a public key, causing all peers to reject future events from that key |

---

### Module 2: Peer-to-Peer Networking (`kanban-net`)

#### MVP Features
| Feature | Description | Design Decision |
|---|---|---|
| **Iroh endpoint** | Long-lived Iroh endpoint running as Tokio task. Binds to random port, registers with Iroh relay for discoverability | Use `iroh::Endpoint` with default relay servers (US/EU/Asia). Allow user to configure custom relay |
| **WAN connection** | QUIC-NAT-Traversal (QNT) for hole-punching. Automatic relay fallback if direct connection fails | Iroh handles this natively. E2E encrypted via TLS 1.3 |
| **LAN discovery** | mDNS via Iroh's local discovery or `mdns` crate. Peers on same network auto-discover within seconds | Must be toggleable (corporate networks may block mDNS) |
| **Gossip protocol** | `iroh-gossip` for pub/sub. Each board has a gossip topic = `hash(board_id)`. New Automerge changes broadcast to all subscribers | Use epidemic broadcast trees (HyParView + PlumTree) built into iroh-gossip |
| **Sync on connect** | When a new peer joins a board's gossip topic, trigger Automerge sync protocol exchange to reconcile state | Automerge's built-in sync is message-based: generate sync message → send → receive reply → repeat until converged |
| **Connection status** | CLI: `app-cli status` shows connected peers, last sync time, pending changes. GUI: green/red indicator + peer list | Poll Iroh endpoint state every 2s for GUI updates |

#### Missing from Original Spec (Added)
| Feature | Why It Matters |
|---|---|
| **Board-scoped networking** | Peers should only exchange data for boards they share. Original spec implies all peers see all data — this is a privacy violation. Each board = separate gossip topic + separate Automerge document |
| **Invite flow** | Joining a board requires: (1) board creator generates invite token containing `board_id + creator's NodeID + optional relay hint`, (2) invitee runs `app-cli board join <token>`, (3) invitee connects to creator's node, (4) creator's node sends full Automerge document. The invite token should be a compact string pasteable in chat/email |
| **Relay self-hosting** | For organizations wanting full sovereignty, document how to run a private Iroh relay server |
| **Bandwidth management** | Throttle gossip broadcast rate to prevent flooding on slow connections. Configurable in `~/.config/p2p-kanban/config.toml` |
| **Peer authentication** | After connecting, peers must prove they are members of the board before receiving data. Challenge-response: peer signs a nonce with their key; verifier checks key is in the board's member list |

---

### Module 3: Data Storage & Event Sourcing (`kanban-storage`)

#### MVP Features
| Feature | Description | Design Decision |
|---|---|---|
| **Automerge document storage** | Each board is one Automerge document. Stored as a single binary blob in SQLite | Table: `boards(board_id TEXT PK, automerge_doc BLOB, last_modified INTEGER, last_heads TEXT)` |
| **Hybrid Logical Clocks** | HLC timestamps on every signed event envelope. Combines wall-clock time with logical counter | **Replaces vector clocks from original spec.** HLC is O(1) per event vs vector clock O(N). Provides real-world timestamps for the activity sidebar while maintaining causal ordering |
| **Conflict resolution** | Handled entirely by Automerge: maps use LWW per-key, lists use RGA (Replicated Growable Array) for ordering, counters use PN-Counter | **No custom CRDT implementation needed.** Automerge handles card ordering (list CRDT), card fields (map CRDT), and concurrent edits natively |
| **State materialization** | `automerge::AutoCommit::document()` returns current state as a JSON-like structure. No manual replay needed — Automerge maintains materialized view incrementally | This eliminates the "replay from start" concern. Automerge is already incremental |
| **Idempotency** | Automerge changes are content-addressed by their hash. Applying the same change twice is a no-op | Built into Automerge — no custom logic needed |
| **Snapshots** | Automerge `save()` produces a compact binary that includes the full history. `save_with_options(SaveOptions::new().deflate())` for compression | Store compressed snapshot in SQLite. On startup, load snapshot → ready in <100ms for boards with 10K events |

#### Missing from Original Spec (Added)
| Feature | Why It Matters |
|---|---|
| **SQLite WAL mode** | Enable Write-Ahead Logging for concurrent read/write without blocking. Critical for UI responsiveness while sync is happening |
| **Database encryption** | Optional SQLCipher support for encrypting the local database at rest. Important for corporate users |
| **Incremental save** | Automerge supports incremental saves (only new changes since last save). Use this for frequent auto-saves, full compaction on a schedule |
| **Corruption recovery** | If SQLite file is corrupted, the system should be able to re-sync the board from peers. Store a list of known peers per board to enable recovery |
| **Data directory** | `~/.local/share/p2p-kanban/` on Linux, `~/Library/Application Support/p2p-kanban/` on macOS, `%APPDATA%\p2p-kanban\` on Windows. Use `dirs` crate |

---

### Module 4: Kanban Core Logic (`kanban-core`)

#### MVP Features — Board Operations
| Operation | Automerge Structure | Notes |
|---|---|---|
| **Create board** | New Automerge document with root map: `{ id, title, columns: [], members: {}, created_at, created_by }` | `board_id` = UUID v7 (time-sortable) |
| **Rename board** | Update `doc.title` | LWW — last writer wins |
| **Delete board (tombstone)** | Set `doc.deleted = true, doc.deleted_at = hlc, doc.deleted_by = pubkey` | Tombstoned boards hidden from UI but retained for sync consistency. Actual data deletion via explicit `prune` |
| **Share board** | Generate invite token: `base58(cbor({ board_id, creator_node_id, relay_hint?, expiry? }))` | Compact enough to paste in Slack/email. Optional expiry for security |

#### MVP Features — Column Operations
| Operation | Automerge Structure | Notes |
|---|---|---|
| **Create column** | `doc.columns.push({ id, title, card_ids: [] })` | Automerge list CRDT handles concurrent inserts |
| **Rename column** | Update `columns[idx].title` | LWW |
| **Delete column** | Remove from `doc.columns` list + tombstone cards in it, or move cards to a "No Column" holding area | Must decide: cascade-delete cards or orphan them. **Recommendation: move cards to first remaining column** |
| **Reorder columns** | Move element within `doc.columns` Automerge list | Automerge RGA handles concurrent reorders. May produce unexpected but valid orderings — acceptable for MVP |

#### MVP Features — Card Operations
| Operation | Automerge Structure | Notes |
|---|---|---|
| **Create card** | Card object in `doc.cards` map (keyed by card_id). Card ID added to parent column's `card_ids` list | Cards stored in a flat map, referenced by ID from columns. This allows moving cards between columns without duplicating data |
| **Edit title/description** | Update `doc.cards[card_id].title` or `.description` | LWW per field via Automerge map |
| **Delete card (tombstone)** | Set `doc.cards[card_id].deleted = true`. Remove card_id from column's `card_ids` list | Tombstone for sync consistency |
| **Archive card** | Set `doc.cards[card_id].archived = true`. Remove from column's `card_ids` | Archived cards visible in a separate "Archive" view |
| **Move card** | Remove card_id from source column's `card_ids`, insert at position in target column's `card_ids` | Two Automerge list operations. Concurrent moves to different columns both succeed — card appears in whichever column processed last (acceptable) |
| **Assign user** | `doc.cards[card_id].assignees = [pubkey1, pubkey2]` | Multi-value set using Automerge list |
| **Labels/tags** | `doc.cards[card_id].labels = ["bug", "urgent"]` | Predefined label colors stored at board level: `doc.label_definitions = { "bug": { color: "#ff0000" } }` |
| **Due date** | `doc.cards[card_id].due_date = "2026-04-15"` | ISO 8601 string. LWW |
| **Checklists** | `doc.cards[card_id].checklists = [{ title, items: [{ text, checked }] }]` | Nested Automerge structure. Concurrent checkbox toggling handled by LWW per item |
| **Comments** | `doc.cards[card_id].comments = [{ id, author, text, created_at }]` | Append-only list. Comments are never edited in MVP (avoids complex CRDT for text within comments) |

#### Missing from Original Spec (Added)
| Feature | Why It Matters |
|---|---|
| **Card comments** | Essential for collaboration. Without comments, users have no way to discuss tasks within the tool |
| **Checklists on cards** | Trello's most-used feature after basic cards. Necessary for parity |
| **Board member list** | The original spec mentions "assign users" but never defines how the board knows who its members are. Must maintain `doc.members` map |
| **Card search/filter** | CLI: `app-cli card search --board <id> --query "deploy"`. GUI: filter bar. Operates on materialized state, not event log |
| **Board templates** | Create a new board pre-populated with columns (e.g., "To Do / In Progress / Done"). Stored as a local template file |

---

### Module 5: Command Line Interface (`kanban-cli`)

#### Command Reference
```
app-cli init                                    # Generate identity, create config
app-cli identity show                           # Show public key and alias
app-cli identity export --passphrase <pass>     # Encrypted key backup
app-cli identity import <file> --passphrase     # Restore key from backup

app-cli board create <title>                    # Create new board
app-cli board list [--json]                     # List all boards
app-cli board view <board_id> [--json]          # Show board state
app-cli board rename <board_id> <new_title>
app-cli board delete <board_id>
app-cli board invite <board_id>                 # Generate invite token
app-cli board join <invite_token>               # Join board from token
app-cli board export <board_id> -o <file>       # Export for air-gapped sync
app-cli board import <file>                     # Merge imported board data

app-cli column create <board_id> <title>
app-cli column rename <board_id> <col_id> <new_title>
app-cli column delete <board_id> <col_id>
app-cli column move <board_id> <col_id> --position <idx>

app-cli card create <board_id> <col_id> <title>
app-cli card edit <board_id> <card_id> --title "..." --desc "..."
app-cli card move <board_id> <card_id> --to-column <col_id> --position <idx>
app-cli card assign <board_id> <card_id> --user <pubkey>
app-cli card label <board_id> <card_id> --add "bug" --remove "wontfix"
app-cli card delete <board_id> <card_id>
app-cli card search <board_id> --query "deploy"

app-cli log view <board_id> [--limit 50] [--json]   # Audit trail
app-cli log state <board_id> --at <timestamp>        # Time-travel snapshot

app-cli daemon start                            # Start P2P networking daemon
app-cli daemon stop
app-cli status [--json]                         # Network health + sync status

app-cli prune <board_id> --before <timestamp>   # Compact old history
app-cli config set <key> <value>                # Modify config
```

#### Design Decisions
| Decision | Rationale |
|---|---|
| **`--json` flag on every command** | Enables scripting and integration. The Tauri app uses Tauri commands (not CLI), but power users can build custom tooling |
| **Daemon as separate process** | The P2P daemon runs as a background process (`app-cli daemon start`). The Tauri app starts it automatically. CLI commands communicate with the daemon via a local Unix socket / named pipe |
| **Subcommand structure** | `board`, `column`, `card`, `log`, `daemon` — mirrors domain model, easy to discover |
| **Human-readable defaults** | Tables with `tabled`, colors with `colored`. JSON only when explicitly requested |

---

### Module 6: Pruning & Storage Management

#### MVP Features
| Feature | Description | Design Decision |
|---|---|---|
| **History compaction** | `app-cli prune <board_id> --before <timestamp>` calls `Automerge::save()` with compaction. Old change history is collapsed into a single snapshot | Automerge supports this natively. The compacted document is semantically identical but smaller |
| **Shallow sync** | When syncing with a new peer, send the compacted document (current state) rather than full history | Automerge sync protocol handles this — new peers receive the save blob, which includes enough information to begin incremental sync from that point |
| **Storage metrics** | `app-cli storage stats` shows per-board storage usage, event count, last compaction date | Helps users understand when pruning is needed |

#### Design Decision (Corrected from Original Spec)
The original spec described "retaining cryptographic hashes after pruning payload data." This is unnecessary with Automerge — compaction produces a valid document with full integrity. There is no separate "hash chain" to maintain. Automerge's internal Merkle DAG ensures integrity.

---

### Module 7: Tauri GUI (`kanban-tauri`)

#### MVP Features
| Feature | Description | Implementation |
|---|---|---|
| **Board dashboard** | Grid/list of boards with title, member count, last activity | Tauri command: `get_boards()` → returns `Vec<BoardSummary>` |
| **Kanban view** | Columns as vertical lanes, cards as draggable items | SolidJS with `@thisbeyond/solid-dnd`. Column and card rendering from reactive store |
| **Drag and drop** | Cards between columns, columns reorder | On drop: call Tauri command `move_card()` or `move_column()`. Optimistic UI update, reconcile on CRDT confirmation |
| **Real-time updates** | Peer changes appear without refresh | Tauri Channel streams CRDT patches from Rust backend. SolidJS `createSignal` / `createStore` updated reactively |
| **Activity sidebar** | Timestamped list of recent actions with user aliases | Derived from Automerge change metadata (author + timestamp + change type) |
| **Network status** | Green dot = daemon running + peers connected. Red = no peers. Yellow = syncing | Tauri command `get_network_status()` polled every 2s, or event-driven via Channel |
| **Card detail modal** | Click card → modal with full description, comments, checklist, labels, assignees, due date | Standard modal component. All edits call Tauri commands |
| **Board settings** | Rename, view members, generate invite, manage labels | Settings panel accessible from board header |
| **Search/filter bar** | Filter cards by text, label, assignee | Client-side filter on materialized state (already in memory) |

#### Post-MVP / Advanced
| Feature | Description |
|---|---|
| **Time-travel slider** | Visual rewind using `Automerge::load()` at specific heads. Slider maps to change history |
| **Keyboard shortcuts** | `n` = new card, `e` = edit, arrow keys for navigation |
| **Theme** | Dark/light mode toggle. Stored in local preferences |
| **Multi-board view** | Split view showing multiple boards side by side |
| **Notifications** | System tray notifications for assignments and mentions |

#### GUI ↔ Backend Communication (Corrected from Original Spec)

**Original spec:** GUI shells out to CLI via `std::process::Command` and parses JSON stdout.

**Corrected:** The GUI imports `kanban-core`, `kanban-storage`, `kanban-net`, and `kanban-crypto` as library dependencies. Communication uses Tauri's native IPC:

- **Tauri Commands** for request/response (user actions → backend → result)
- **Tauri Channels** for streaming (backend → frontend real-time updates)

This gives us type-safe communication, no serialization overhead beyond Tauri's built-in serde, no process spawning, and the ability to share in-memory state (e.g., the Automerge document handle) across commands.

---

## Non-Functional Requirements & Edge Cases

### Performance
| Scenario | Target | Strategy |
|---|---|---|
| Board with 10,000 cards | Open in <2s, smooth scrolling | Virtual scrolling in GUI (render only visible cards). Automerge handles large docs efficiently |
| 50 concurrent peers | Sync within 5s of change | Iroh gossip dissemination is O(log N). Automerge sync is incremental |
| Offline for 1 week, 500 changes on each side | Merge in <10s | Automerge sync protocol is designed for this. Binary delta exchange, not full document |
| Cold start (app launch) | Ready in <3s | Load Automerge snapshot from SQLite. No event replay needed |

### Security
| Concern | Mitigation |
|---|---|
| Rogue peer sends massive document | Size limits on incoming Automerge sync messages (configurable, default 100MB). Reject and disconnect |
| Replay attack (re-sending old valid events) | Automerge deduplicates by change hash. No-op |
| Man-in-the-middle | Iroh uses TLS 1.3 end-to-end. Peer identity is their public key, verified during QUIC handshake |
| Denial-of-service via connection flooding | Rate-limit new connections per IP. Iroh relay servers don't expose user IPs to each other |
| Local database theft | Optional SQLCipher encryption. OS keychain for private key |
| Invite token interception | Tokens can have expiry. Board creator can revoke invites. Consider adding a one-time-use flag |

### Reliability
| Scenario | Behavior |
|---|---|
| App crashes mid-write | SQLite WAL mode ensures atomic writes. Automerge document is either fully committed or not |
| Two users delete the same card simultaneously | Both tombstone events apply. Card is deleted. No conflict |
| User moves card to column A; other user moves same card to column B simultaneously | Both moves apply in causal order (HLC). Card ends up in whichever column was "later." This is the expected LWW behavior. GUI shows the reconciled state |
| Network partition for days | Both sides operate independently. On reconnect, Automerge merge produces convergent state. No data loss |
| Disk full | SQLite returns error. Application catches it, shows "disk full" warning, disables writes. Read-only mode continues working |

---

## Phased Roadmap

### Phase 0: Project Scaffolding — 1 week
**Goal:** Workspace structure, CI, tooling

- Set up Cargo workspace with all 6 crates (empty stubs)
- Configure CI (GitHub Actions): clippy, fmt, test, build for macOS/Linux/Windows
- Set up Tauri v2 app skeleton with SolidJS
- Configure SQLite with migrations (via `rusqlite` + `refinery`)
- Establish error handling pattern (`thiserror` for library crates, `anyhow` for binaries)
- Add `tracing` for structured logging across all crates

**Definition of done:** `cargo build --workspace` succeeds. Tauri dev server launches. CI green.

### Phase 1: Local Single-User Kanban — 3-4 weeks
**Goal:** A fully functional local Kanban board with no networking

- `kanban-crypto`: Keypair generation, signing, verification
- `kanban-core`: Board, column, card CRDT operations using Automerge
- `kanban-storage`: SQLite persistence (save/load Automerge documents)
- `kanban-cli`: All `board`, `column`, `card` commands (local only)
- Tauri GUI: Board dashboard, Kanban view with drag-and-drop, card detail modal
- Property-based tests: Create 1000 random operations, verify materialized state is valid

**Key risk:** Automerge API learning curve. Mitigate with a spike/prototype in week 1.

**Definition of done:** User can create boards, add columns/cards, drag cards, edit details — all persisted across app restart. CLI and GUI both work.

### Phase 2: P2P Networking — 3-4 weeks
**Goal:** Two peers can share and sync a board in real-time

- `kanban-net`: Iroh endpoint setup, gossip topic per board
- Invite flow: generate token, join board, initial sync
- Automerge sync protocol integration (delta exchange over Iroh streams)
- Real-time UI updates via Tauri Channels
- LAN discovery (mDNS)
- WAN hole-punching (Iroh QNT)
- `daemon start/stop` commands
- Network status in CLI and GUI
- Integration test: 3-node network, concurrent edits, verify convergence

**Key risk:** Iroh 0.96 has a known hole-punching regression on network change. Mitigate with relay fallback and monitoring Iroh releases.

**Definition of done:** Two users on different machines can share a board via invite token, make concurrent edits, and see each other's changes appear within 2s.

### Phase 3: Hardening & Features — 2-3 weeks
**Goal:** Production-quality reliability and missing features

- Comments on cards
- Checklists
- Card search and filter
- Audit trail / activity log
- Key backup/restore
- Board export/import (air-gapped sync)
- Pruning / compaction
- Error handling audit: every error path produces a user-friendly message
- Crash recovery testing
- Performance profiling (10K cards, 50 peers)
- SQLCipher integration (optional encrypted storage)

**Definition of done:** All features work end-to-end. No panics in normal usage. Performance targets met.

### Phase 4: Polish & Release — 2 weeks
**Goal:** Ship a public alpha

- macOS/Linux/Windows packaging (Tauri bundler: `.dmg`, `.AppImage`, `.msi`)
- Auto-update mechanism (Tauri updater plugin)
- First-run onboarding flow in GUI (create identity → create first board)
- Settings page (alias, avatar, relay config, LAN discovery toggle)
- Dark/light theme
- README, docs site, quickstart guide
- Release automation (GitHub Actions: build → sign → publish)

**Definition of done:** Downloadable binary works on all three platforms. New user can install, create a board, and invite a peer within 5 minutes.

---

## Risk Register

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | Iroh makes breaking wire-protocol change before 1.0 | High | High | Pin Iroh version. All peers must use compatible version. Build version negotiation into handshake |
| 2 | Automerge document grows unbounded for active boards | Medium | High | Compaction strategy in Phase 3. Monitor document size. Alert user when board exceeds 50MB |
| 3 | NAT hole-punching fails on restrictive corporate networks | Medium | Medium | Iroh relay fallback is automatic. Document self-hosted relay setup for enterprise users |
| 4 | CRDT merge produces unexpected card ordering after concurrent reorders | Medium | Low | Acceptable for MVP — the result is valid, just potentially surprising. Add "manual reorder" button as escape hatch |
| 5 | Ed25519 key loss means permanent identity loss | Medium | High | Key backup/restore in Phase 3. Prominent warning in onboarding UI |
| 6 | Large number of peers (>50) causes gossip flood | Low | Medium | Iroh gossip uses epidemic broadcast trees (logarithmic dissemination). Add configurable max-peers limit |
| 7 | Clock skew causes HLC ordering anomalies | Low | Low | HLC is designed to handle moderate clock skew. Log warnings when skew exceeds 5 minutes |
| 8 | SQLite file corruption on power loss | Low | High | WAL mode + regular checkpoint. Can re-sync from peers as recovery mechanism |

---

## Open Questions

These require decisions from the project team before implementation:

1. **Project name:** "p2p-kanban" is a placeholder. The name affects crate names, binary name, config directory, etc. Decide before Phase 0.
2. **License:** MIT? Apache 2.0? AGPL? Affects who can use and extend the project.
3. **Minimum supported peer count:** Is the target 2-5 (team) or 50-100 (organization)? Affects gossip tuning and testing strategy.
4. **Card description format:** Plain text only, or Markdown? Markdown adds rendering complexity in the GUI but is expected by developers.
5. **Self-hosted relay:** Should the project provide and operate public relay servers, or require users to self-host? Budget implications.
6. **Mobile roadmap:** Is mobile a goal for v2? If yes, the Automerge document format and sync protocol will need to be compatible with a future mobile client (likely React Native + automerge-js).
7. **Board access control:** MVP has no roles (all members are equal). Should v1.1 add admin/member/viewer roles? This adds significant CRDT complexity (permission checks on merge).

---

## Assumptions

- Developers implementing this plan are comfortable with Rust and async programming (Tokio).
- Target users are technical teams (developers, designers) comfortable installing a desktop app.
- Boards are small-to-medium (1-500 cards per board). Boards with 10K+ cards are edge cases, not the primary use case.
- Peers are generally cooperative (no Byzantine fault tolerance needed). A malicious peer can be banned, but the system does not protect against a peer who has the private key and intentionally corrupts data.
- Internet connectivity is intermittent but not absent for extended periods (weeks of offline usage is supported but not the primary mode).
- The project is open-source and community-driven (no monetization pressure affecting architecture decisions).
