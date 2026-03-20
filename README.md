<div align="center">

<img src="assets/logo.svg" width="160" alt="MonoTask monkey mascot"/>

# monotask

**Peer-to-peer kanban — no server, no account, no nonsense.**

Boards live on your machine. Spaces connect people via cryptographic invites.
Everything is signed with Ed25519. Nothing phones home.

[![Latest Release](https://img.shields.io/github/v/release/nokhodian/monotask?style=flat-square&color=f5a623)](https://github.com/nokhodian/monotask/releases/latest)
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
| 🖥️ **Desktop GUI** | Tauri v2 app with drag-and-drop kanban, labels, due dates, assignees, activity feed |
| ⌨️ **CLI** | Everything scriptable with `--json` output; full `ai-help` reference for agents |
| 🔄 **CRDT sync** | Automerge-based — concurrent edits never conflict |
| 📦 **No dependencies** | One binary, one SQLite file, done |

---

## Install

### Desktop App — Homebrew Cask (macOS, recommended)

```bash
brew tap nokhodian/tap
brew install --cask monotask
```

Opens as a native macOS app. Double-click `Monotask.app` in Applications.

### CLI — Homebrew Formula

```bash
brew tap nokhodian/tap
brew install monotask
```

### Desktop App — Direct DMG download

Grab the DMG from the [Releases page](https://github.com/nokhodian/monotask/releases/latest), open it, and drag `Monotask.app` to Applications.

| Platform | Download |
|----------|----------|
| macOS Apple Silicon (Desktop) | `Monotask-<version>-aarch64.dmg` |

### CLI — Direct binary download

| Platform | Archive |
|----------|---------|
| macOS Apple Silicon | `monotask-<version>-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `monotask-<version>-x86_64-apple-darwin.tar.gz` |
| Linux x86_64 (musl) | `monotask-<version>-x86_64-linux.tar.gz` |

```bash
# Example: macOS Apple Silicon CLI
curl -L https://github.com/nokhodian/monotask/releases/latest/download/monotask-v0.1.0-aarch64-apple-darwin.tar.gz | tar xz
mv monotask /usr/local/bin/
monotask --help
```

SHA-256 checksums are provided as `.sha256` sidecar files next to each download.

### Build from source

```bash
git clone https://github.com/nokhodian/monotask.git
cd monotask

# CLI only
cargo build -p kanban-cli --release
cp target/release/app-cli /usr/local/bin/monotask

# Desktop app (requires Tauri CLI and Node.js)
cd crates/kanban-tauri
cargo tauri build
```

---

## Architecture

```
monotask/
├── crates/
│   ├── kanban-core/      # Domain model (boards, cards, CRDTs via Automerge)
│   ├── kanban-crypto/    # Ed25519 identity, signing, invite tokens
│   ├── kanban-storage/   # SQLite persistence (boards, spaces, invites)
│   ├── kanban-net/       # P2P networking layer
│   ├── kanban-cli/       # CLI frontend (clap) → binary: app-cli / monotask
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

## CLI Reference

The CLI binary is named `monotask` (release) or `app-cli` (when run via `cargo run`).

### Identity & Profile

```bash
monotask init                            # Initialize local identity
monotask profile show                    # Show pubkey, display name, avatar
monotask profile set-name "Ada"          # Set display name
monotask profile set-avatar photo.png    # Set avatar
monotask profile import-ssh-key          # Import from ~/.ssh/id_ed25519
monotask profile import-ssh-key key.pem  # Import from custom path
```

### Boards

```bash
monotask board create "My Board" [--json]
monotask board list [--json]
```

### Columns & Cards

```bash
monotask column create <board-id> "To Do" [--json]
monotask column list <board-id> [--json]

monotask card create <board-id> <col-id> "Fix the thing" [--json]
monotask card view <board-id> <card-id> [--json]
monotask card comment add <board-id> <card-id> "looks good" [--json]
monotask card comment list <board-id> <card-id> [--json]
monotask card comment delete <board-id> <card-id> <comment-id> [--json]
```

### Checklists

```bash
monotask checklist add <board-id> <card-id> "QA checklist" [--json]
monotask checklist item-add <board-id> <card-id> <checklist-id> "Write tests" [--json]
monotask checklist item-check <board-id> <card-id> <checklist-id> <item-id>
monotask checklist item-uncheck <board-id> <card-id> <checklist-id> <item-id>
```

### Spaces

```bash
# Create and inspect
monotask space create "Team Alpha"
monotask space list
monotask space info <space-id>

# Invite flow
monotask space invite generate <space-id>              # Print token
monotask space invite export <space-id> invite.space   # Write .space file
monotask space invite revoke <space-id>                # Invalidate token

# Join
monotask space join <token-or-path-to-.space-file>

# Boards in a space
monotask space boards add <space-id> <board-id>
monotask space boards remove <space-id> <board-id>
monotask space boards list <space-id>

# Members
monotask space members list <space-id>
monotask space members kick <space-id> <pubkey>
```

---

## Invite Flow

```
1. Host:   monotask space invite generate <id>        →  base58-token
2. Host:   monotask space invite export <id> x.space  →  writes x.space file
3. Guest:  monotask space join x.space                →  joined!
4. Host:   monotask space members list <id>           →  [ you, guest ]
```

QR code support is built into the desktop GUI — useful for in-person sharing without copy-pasting a token.

---

## Desktop GUI

Run with `cargo tauri dev` from `crates/kanban-tauri/`.

- **Spaces sidebar** — switch spaces, create new ones, join via token
- **Kanban board** — drag-and-drop cards between columns; delete columns
- **Card detail** — title, description, labels, due date (inline picker on tile), assignee from space members
- **Activity feed** — movement history with HLC timestamps + comment thread
- **Profile modal** — set name, avatar, import SSH key

---

## Security Model

| Concern | Approach |
|---|---|
| Identity | Ed25519 keypair, stored locally, never leaves device |
| Invite tokens | Signed by space host, single-use per generation, revocable |
| Token deduplication | SHA-256 hash as primary key |
| No tracking | Zero telemetry, zero analytics, zero network calls at rest |

---

## AI Agent Onboarding

MonoTask ships a built-in reference document designed for AI agents and automation. One command gives a complete context dump: every command, every flag, JSON schemas, ID formats, storage layout, common workflows, and known limitations.

```bash
monotask ai-help
```

### Quickest onboarding prompt

Paste this at the start of any AI agent session that needs to manage MonoTask:

```
You have access to the `monotask` CLI for task management.
First, run this command and read the output carefully:

  monotask ai-help

Then proceed with the user's request. Key rules:
- Always use --json for machine-readable output
- Board/column/card IDs are UUIDs — use the full UUID in all commands
- Card numbers like "a7f3-1" are display-only; commands require the UUID
- Data lives in ~/.local/share/p2p-kanban/ by default; use --data-dir to override
```

### What `ai-help` covers

| Section | Contents |
|---------|----------|
| Quick-start | 6-step checklist for first run |
| Global flags | `--data-dir`, what the data directory contains |
| Identity | Key resolution order, how pubkeys are used |
| Every command | Exact syntax, all flags, text vs JSON output, full JSON schemas |
| Timestamps | HLC format with JavaScript parsing snippet |
| ID formats | UUID vs human card number distinction |
| Storage | SQLite table schema, Automerge document structure |
| Workflows | Shell scripts: create+populate board, inspect all cards, collaborative setup, checklists, comments |
| Limitations | No auto-sync between peers, CLI-only quirks, known gaps |

### Example: agent creates and populates a board

```bash
# Agent reads the reference first
monotask ai-help

# Create a sprint board
BOARD=$(monotask board create "Sprint 1" --json | jq -r .id)
TODO=$(monotask column create $BOARD "Todo" --json | jq -r .id)
DOING=$(monotask column create $BOARD "Doing" --json | jq -r .id)
DONE=$(monotask column create $BOARD "Done" --json | jq -r .id)

# Populate cards
for task in "Design API" "Write tests" "Deploy"; do
  monotask card create $BOARD $TODO "$task" --json
done

# Read current state
monotask column list $BOARD --json | jq '.[] | {col: .title, count: (.card_ids | length)}'
```

### Example: agent reviews and comments on all cards

```bash
BOARD="<board-uuid>"

# Get all column + card data
COLS=$(monotask column list $BOARD --json)

# For each card, view it and add a comment
echo $COLS | jq -r '.[].card_ids[]' | while read CARD_ID; do
  CARD=$(monotask card view $BOARD $CARD_ID --json)
  TITLE=$(echo $CARD | jq -r .title)
  echo "Reviewing: $TITLE"
  monotask card comment add $BOARD $CARD_ID "Reviewed by agent on $(date -u +%Y-%m-%d)"
done
```

---

## Building & Development

```bash
# Run all tests
cargo test --workspace

# Check everything compiles
cargo check --workspace

# Run CLI in dev mode
cargo run -p kanban-cli -- ai-help

# Run desktop app in dev mode
cd crates/kanban-tauri && cargo tauri dev
```

### Cutting a release

Releases are automated via `.github/workflows/release.yml`. Push a version tag and CI builds macOS arm64 + x86_64 and Linux x86_64 (musl static), creates a GitHub release, and updates the Homebrew formula in `nokhodian/homebrew-tap` automatically.

```bash
git tag v0.2.0
git push origin v0.2.0
# CI builds all platforms, creates release, updates Homebrew formula
```

> **Note:** The auto-update step requires a `HOMEBREW_TAP_TOKEN` secret set in the repo's GitHub Actions settings with write access to `nokhodian/homebrew-tap`.

---

<div align="center">

Made with ☕ and Rust · [MIT License](LICENSE)

<sub>The monkey approves of your task management.</sub>

</div>
