<div align="center">

<img src="assets/logo.svg" width="160" alt="MonoTask monkey mascot"/>

# monotask

**Peer-to-peer kanban — no server, no account, no nonsense.**

Boards live on your machine. Spaces connect people via cryptographic invites.
Everything is signed with Ed25519. Nothing phones home.

[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Tauri](https://img.shields.io/badge/tauri-v2-24c8db?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app)
[![SQLite](https://img.shields.io/badge/storage-sqlite-003b57?style=flat-square&logo=sqlite&logoColor=white)](https://sqlite.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-f5a623?style=flat-square)](LICENSE)

</div>

---

## What is MonoTask?

MonoTask is a **local-first, peer-to-peer kanban board** built in Rust. You own your data. Boards are stored in a local SQLite database and synced using [Automerge](https://automerge.org/) CRDTs — meaning concurrent edits merge automatically without a central server.

**Spaces** let you group boards and share them with others through signed invite tokens. No registration. No cloud. You invite someone, they join, done.

```
you ──── [invite token] ──→ teammate
         (Ed25519 signed)
              ↓
    [joined your Space]
    [can see your boards]
```

---

## Features

| | |
|---|---|
| 🃏 **Full Kanban** | Boards → Columns → Cards → Checklists → Comments |
| 🔐 **Cryptographic identity** | Ed25519 keypair, generated locally or imported from SSH |
| 🌐 **Spaces** | Shared workspaces with invite/revoke/kick flows |
| 📋 **QR invites** | Generate QR codes for invite tokens — works offline |
| 🖥️ **Desktop GUI** | Tauri v2 app with full Space + Board management UI |
| ⌨️ **CLI** | Everything the GUI does, scriptable, JSON output |
| 🔄 **CRDT sync** | Automerge-based — concurrent edits never conflict |
| 📦 **No dependencies** | One binary, one SQLite file, done |

---

## Architecture

```
monotask/
├── crates/
│   ├── kanban-core/      # Domain model (boards, cards, CRDTs via Automerge)
│   ├── kanban-crypto/    # Ed25519 identity, signing, invite tokens
│   ├── kanban-storage/   # SQLite persistence (boards, spaces, invites)
│   ├── kanban-net/       # P2P networking layer
│   ├── kanban-cli/       # CLI frontend (clap)
│   └── kanban-tauri/     # Desktop GUI (Tauri v2 + vanilla JS)
```

**Data flow:**

```
CLI / GUI
    │
    ▼
kanban-core   ←── kanban-crypto (signing, keys)
    │
    ▼
kanban-storage (SQLite + Automerge docs)
    │
    ▼
kanban-net (P2P sync, in progress)
```

---

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) 1.78+
- [Tauri CLI](https://tauri.app/v1/guides/getting-started/prerequisites) (for GUI only)
- Node.js (for Tauri frontend tooling)

### Build the CLI

```bash
cargo build -p kanban-cli --release
# binary at: target/release/kanban-cli
```

### Run the Desktop App

```bash
cd crates/kanban-tauri
cargo tauri dev
```

---

## CLI Reference

### Identity

```bash
kanban-cli init                          # Initialize local identity
kanban-cli profile show                  # Show your public key + name
kanban-cli profile set-name "Ada"        # Set display name
kanban-cli profile set-avatar photo.png  # Set avatar (base64-encoded)
kanban-cli profile import-ssh-key        # Import from ~/.ssh/id_ed25519
```

### Boards

```bash
kanban-cli board create "My Board"
kanban-cli board list [--json]
```

### Columns & Cards

```bash
kanban-cli column create <board-id> "To Do"
kanban-cli column list <board-id>

kanban-cli card create <board-id> <col-id> "Fix the thing"
kanban-cli card view <board-id> <card-id>
kanban-cli card comment add <board-id> <card-id> "looks good"
kanban-cli card comment list <board-id> <card-id>
```

### Checklists

```bash
kanban-cli checklist add <board-id> <card-id> "QA checklist"
kanban-cli checklist item-add <board-id> <card-id> <checklist-id> "Write tests"
kanban-cli checklist item-check <board-id> <card-id> <checklist-id> <item-id>
kanban-cli checklist item-uncheck <board-id> <card-id> <checklist-id> <item-id>
```

### Spaces

```bash
# Create and inspect
kanban-cli space create "Team Alpha"
kanban-cli space list
kanban-cli space info <space-id>

# Invite flow
kanban-cli space invite generate <space-id>         # Print token
kanban-cli space invite export <space-id> invite.space  # Write .space file
kanban-cli space invite revoke <space-id>            # Invalidate current token

# Join
kanban-cli space join <token-or-path-to-.space-file>

# Boards
kanban-cli space boards add <space-id> <board-id>
kanban-cli space boards remove <space-id> <board-id>
kanban-cli space boards list <space-id>

# Members
kanban-cli space members list <space-id>
kanban-cli space members kick <space-id> <pubkey>
```

---

## Invite Flow

Invites are signed Ed25519 tokens serialised as JSON (`.space` files).

```
1. Host:   space invite generate <id>     →  eyJ0eXAiOiJzcGFjZS1pbnZpdGUiLC...
2. Host:   space invite export <id> x.space  →  writes x.space (shareable file)
3. Guest:  space join x.space             →  joined!
4. Host:   space members list <id>        →  [ you, guest ]
```

QR code support is built into the desktop GUI — useful for in-person sharing without copy-pasting a token.

---

## Desktop GUI

The Tauri app exposes the full feature set through a sidebar-first interface:

- **Spaces sidebar** — switch spaces, create new ones, join via token
- **Members tab** — see who's in the space, kick if needed
- **Boards tab** — add/remove boards from a space
- **Invite tab** — generate token, display QR code, revoke with one click
- **Profile modal** — set your name, avatar, import SSH key

---

## Security Model

| Concern | Approach |
|---|---|
| Identity | Ed25519 keypair, stored locally, never leaves device |
| Invite tokens | Signed by space host, single-use, revocable |
| Token deduplication | SHA-256 hash as primary key; `INSERT OR REPLACE` handles deterministic re-generation |
| No tracking | Zero telemetry, zero analytics, zero network calls at rest |

---

## Development

```bash
# Run all tests
cargo test --workspace

# Check everything compiles
cargo check --workspace

# Run the CLI
cargo run -p kanban-cli -- --help
```

---

<div align="center">

Made with ☕ and Rust · [MIT License](LICENSE)

<sub>The monkey approves of your task management.</sub>

</div>
