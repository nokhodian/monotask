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

MonoTask is a **local-first, peer-to-peer kanban board** built in Rust. Your boards are stored in a local SQLite database and synced with teammates using [Automerge](https://automerge.org/) CRDTs — concurrent edits merge automatically, no central server required.

**Spaces** let you group boards and share them with others through signed invite tokens. No registration. No cloud. Generate a token, send it, done.

```
you ──── [invite token] ──→ teammate
         (Ed25519 signed)
              ↓
    [joined your Space]
    [sees your boards]
```

MonoTask ships as two things: a **native desktop app** (drag-and-drop kanban UI) and a **CLI** (`monotask`) for scripting, automation, and AI agents.

---

## Quick Start

Get running in under a minute:

**macOS (Homebrew — installs both desktop app and CLI):**
```bash
brew tap nokhodian/tap
brew install monotask          # CLI
brew install --cask monotask   # Desktop app
```

**Everyone else:** see the [Install](#install) section below.

Once installed, open the desktop app from Applications or run:
```bash
monotask board create "My First Board" --json
```

---

## Features

| | |
|---|---|
| 🃏 **Full kanban** | Boards → Columns → Cards → Checklists → Comments |
| 🔐 **Cryptographic identity** | Ed25519 keypair, generated locally or imported from SSH |
| 🌐 **Spaces** | Shared workspaces with invite, revoke, and kick flows |
| 📋 **QR invites** | Generate QR codes for invite tokens — works offline |
| 🖥️ **Desktop app** | Tauri v2 — drag-and-drop, labels, due dates, assignees, cover colors |
| ⌨️ **Full CLI** | Every feature scriptable with `--json` output; `ai-help` for agents |
| 🔄 **CRDT sync** | Automerge-based — concurrent edits never conflict |
| 📦 **Zero dependencies** | One binary, one SQLite file |

---

## Install

### macOS

#### Homebrew (recommended)

```bash
brew tap nokhodian/tap
brew install monotask          # CLI
brew install --cask monotask   # Desktop app
```

> **Upgrading from a DMG install?** If you previously dragged in a `.dmg`, Homebrew won't recognize it. Force-install to fix:
> ```bash
> brew install --cask monotask --force
> ```

> **"Monotask is damaged and can't be opened"** — macOS Gatekeeper shows this for unsigned apps. Run once after installing:
> ```bash
> find /Applications/Monotask.app -print0 | xargs -0 xattr -c
> codesign --force --deep --sign - /Applications/Monotask.app
> ```

#### Direct download (macOS)

| What | Download |
|------|----------|
| Desktop app (Apple Silicon) | `Monotask-<version>-aarch64.dmg` |
| CLI (Apple Silicon) | `monotask-<version>-aarch64-apple-darwin.tar.gz` |
| CLI (Intel) | `monotask-<version>-x86_64-apple-darwin.tar.gz` |

Get all downloads from the [Releases page](https://github.com/nokhodian/monotask/releases/latest).

```bash
# CLI install
curl -L https://github.com/nokhodian/monotask/releases/latest/download/monotask-<version>-aarch64-apple-darwin.tar.gz | tar xz
mv monotask /usr/local/bin/
monotask --help
```

---

### Windows

Download from the [Releases page](https://github.com/nokhodian/monotask/releases/latest):

| What | Download |
|------|----------|
| Desktop app (installer) | `Monotask-<version>-x64-setup.exe` |
| CLI | `monotask-<version>-x86_64-windows.zip` |

Run the installer for the desktop app. For the CLI:

```powershell
$ver = (Invoke-RestMethod https://api.github.com/repos/nokhodian/monotask/releases/latest).tag_name
Invoke-WebRequest "https://github.com/nokhodian/monotask/releases/download/$ver/monotask-$ver-x86_64-windows.zip" -OutFile monotask.zip
Expand-Archive monotask.zip -DestinationPath .
Move-Item monotask.exe "$env:USERPROFILE\bin\monotask.exe"
monotask --help
```

> **Add to PATH (if needed):**
> ```powershell
> [Environment]::SetEnvironmentVariable("Path", $env:Path + ";$env:USERPROFILE\bin", "User")
> ```

> **"Windows protected your PC"** — Windows SmartScreen shows this for unsigned apps. Click **More info → Run anyway**.

---

### Linux

```bash
# x86_64 (musl static binary — works on any distro)
curl -L https://github.com/nokhodian/monotask/releases/latest/download/monotask-<version>-x86_64-linux.tar.gz | tar xz
sudo mv monotask /usr/local/bin/
monotask --help
```

| Download |
|----------|
| `monotask-<version>-x86_64-linux.tar.gz` |

---

### Build from source

Requires: Rust 1.78+, for the desktop app also Tauri CLI and Node.js.

```bash
git clone https://github.com/nokhodian/monotask.git
cd monotask

# CLI
cargo build -p monotask-cli --release
cp target/release/app-cli /usr/local/bin/monotask

# Desktop app
cd crates/monotask-tauri
cargo tauri build
```

SHA-256 checksums are provided as `.sha256` sidecar files next to each release download.

---

## Update

```bash
# Desktop app (Homebrew)
brew upgrade --cask monotask

# CLI (Homebrew)
brew upgrade monotask
```

> After a Homebrew upgrade, if macOS shows "Monotask is damaged", re-run the Gatekeeper fix:
> ```bash
> find /Applications/Monotask.app -print0 | xargs -0 xattr -c
> codesign --force --deep --sign - /Applications/Monotask.app
> ```

The desktop app also checks for updates automatically on launch and prompts you to install them.

---

## CLI Reference

The CLI binary is named `monotask` (installed) or `app-cli` (when built directly via `cargo build`).

For a complete machine-readable reference including all JSON schemas, run:
```bash
monotask ai-help          # full markdown reference
monotask ai-help --json   # structured JSON schema
```

### Profile & Identity

```bash
monotask profile show                    # Show pubkey, name, avatar path
monotask profile set-name "Ada"          # Set display name
monotask profile set-avatar photo.png    # Set avatar image
monotask profile import-ssh-key          # Import from ~/.ssh/id_ed25519
monotask profile import-ssh-key key.pem  # Import from custom path
```

### Boards

```bash
monotask board create "Sprint 1" [--json]
monotask board list [--json]
monotask board rename <board-id> "New Name" [--json]
```

### Columns

```bash
monotask column create <board-id> "To Do" [--json]
monotask column list <board-id> [--json]
monotask column rename <board-id> <col-id> "In Review" [--json]
monotask column delete <board-id> <col-id> [--json]
```

### Cards

```bash
# Create & read
monotask card create <board-id> <col-id> "Fix the thing" [--json]
monotask card view <board-id> <card-id> [--json]

# Rename & lifecycle
monotask card rename <board-id> <card-id> "New title" [--json]
monotask card delete <board-id> <card-id> [--json]
monotask card archive <board-id> <card-id> [--json]
monotask card copy <board-id> <card-id> <target-col-id> [--json]
monotask card move <board-id> <card-id> <to-col-id> [--json]

# Properties
monotask card set-description <board-id> <card-id> "Long description here" [--json]
monotask card set-cover <board-id> <card-id> "#c8962a"  # or "none" to clear
monotask card set-due-date <board-id> <card-id> "2024-12-31"  # or "none"
monotask card set-priority <board-id> <card-id> high  # low | medium | high | none
monotask card set-assignee <board-id> <card-id> <pubkey-hex>  # or "none"

# Labels
monotask card label add <board-id> <card-id> "bug" [--json]
monotask card label remove <board-id> <card-id> "bug" [--json]
monotask card label list <board-id> <card-id> [--json]

# Comments
monotask card comment add <board-id> <card-id> "Looks good" [--json]
monotask card comment list <board-id> <card-id> [--json]
monotask card comment edit <board-id> <card-id> <comment-id> "Updated text" [--json]
monotask card comment delete <board-id> <card-id> <comment-id> [--json]
```

### Checklists

```bash
monotask checklist add <board-id> <card-id> "QA checklist" [--json]
monotask checklist item-add <board-id> <card-id> <cl-id> "Write tests" [--json]
monotask checklist item-check <board-id> <card-id> <cl-id> <item-id> [--json]
monotask checklist item-uncheck <board-id> <card-id> <cl-id> <item-id> [--json]
monotask checklist item-delete <board-id> <card-id> <cl-id> <item-id> [--json]
monotask checklist delete <board-id> <card-id> <cl-id> [--json]
```

### Spaces

```bash
# Create & inspect
monotask space create "Team Alpha"
monotask space list
monotask space info <space-id>

# Invite flow
monotask space invite generate <space-id>              # Print token
monotask space invite export <space-id> invite.space   # Write .space file
monotask space invite revoke <space-id>                # Invalidate token

# Join
monotask space join <token-or-path-to-.space-file>

# Boards
monotask space boards add <space-id> <board-id>
monotask space boards remove <space-id> <board-id>
monotask space boards list <space-id>

# Members
monotask space members list <space-id>
monotask space members kick <space-id> <pubkey>
```

### P2P Sync

```bash
monotask sync                          # Start sync daemon (foreground)
monotask sync --detach                 # Run in background
monotask sync --stop                   # Stop background daemon
monotask sync --status                 # Show daemon status
monotask sync --port 7272              # Listen on a fixed port
monotask sync --peer /ip4/1.2.3.4/tcp/7272  # Dial a specific peer
```

---

## Spaces & Invite Flow

Spaces are shared containers for boards. Anyone with an invite token can join.

```
1. Host:   monotask space invite generate <id>        →  base58-token
2. Host:   monotask space invite export <id> x.space  →  writes x.space file
3. Guest:  monotask space join x.space                →  joined!
4. Host:   monotask space members list <id>           →  [ you, guest ]
```

The desktop app supports QR code generation for in-person sharing without copy-pasting a token.

---

## Desktop App

The desktop GUI covers everything in the CLI and more:

- **Spaces sidebar** — switch spaces, create new ones, join via token or QR code
- **Kanban board** — drag-and-drop cards between columns; fixed-height cards with scroll
- **Card detail** — title, description, labels, due date, cover color, assignee, priority
- **Activity feed** — movement history with timestamps and comment thread
- **Profile modal** — set name, avatar, import SSH key
- **Auto-update** — checks for new releases on launch

Run in development mode:
```bash
cd crates/monotask-tauri
cargo tauri dev
```

---

## AI Agent Onboarding

MonoTask ships a built-in reference designed for AI agents and automation. One command gives everything an agent needs: every command, all flags, JSON schemas, ID formats, storage layout, common workflows, and known quirks.

```bash
monotask ai-help                      # Full markdown reference
monotask ai-help --json               # Structured JSON schema
monotask ai-help --section commands   # Just the command reference
monotask ai-help --section schemas    # Just the JSON output schemas
monotask ai-help --section workflows  # Just the shell script recipes
monotask ai-help --section gotchas    # Just the edge cases and traps
```

### Quickest onboarding prompt

Paste this at the start of any AI session that needs to manage MonoTask:

```
You have access to the `monotask` CLI for task management.
First, run this command and read the output carefully:

  monotask ai-help

Then proceed with the user's request. Key rules:
- Always use --json for machine-readable output
- Board/column/card IDs are UUIDs — use the full UUID in all commands
- Card numbers like "a7f3-1" are display-only; commands require the UUID
- Data lives in ~/.local/share/monotask/ by default; use --data-dir to override
```

### Example: agent creates and populates a board

```bash
monotask ai-help  # read first

BOARD=$(monotask board create "Sprint 1" --json | jq -r .id)
TODO=$(monotask column create $BOARD "Todo" --json | jq -r .id)
DOING=$(monotask column create $BOARD "Doing" --json | jq -r .id)
DONE=$(monotask column create $BOARD "Done" --json | jq -r .id)

for task in "Design API" "Write tests" "Deploy"; do
  monotask card create $BOARD $TODO "$task" --json
done

monotask column list $BOARD --json | jq '.[] | {col: .title, cards: (.card_ids | length)}'
```

### Example: agent reviews all cards and adds comments

```bash
BOARD="<board-uuid>"
COLS=$(monotask column list $BOARD --json)

echo $COLS | jq -r '.[].card_ids[]' | while read CARD_ID; do
  TITLE=$(monotask card view $BOARD $CARD_ID --json | jq -r .title)
  echo "Reviewing: $TITLE"
  monotask card comment add $BOARD $CARD_ID "Reviewed by agent on $(date -u +%Y-%m-%d)"
done
```

---

## Architecture

```
crates/
├── monotask-core/     # Domain model — boards, cards, columns, CRDTs via Automerge
├── monotask-crypto/   # Ed25519 identity, signing, invite token generation
├── monotask-storage/  # SQLite persistence — boards, spaces, invites, card index
├── monotask-net/      # P2P networking — mDNS discovery, libp2p swarm, sync protocol
├── monotask-cli/      # CLI frontend (clap) → binary: app-cli / monotask
└── monotask-tauri/    # Desktop GUI — Tauri v2 + vanilla JS
```

**Data flow:**

```
CLI / Desktop GUI
       │
       ▼
monotask-core  ←── monotask-crypto (signing, keys)
       │
       ▼
monotask-storage (SQLite + Automerge docs)
       │
       ▼
monotask-net (P2P sync)
```

Data is stored in `~/.local/share/monotask/` (Linux/macOS) or `%APPDATA%\monotask\` (Windows). Override with `--data-dir`.

---

## Security Model

| Concern | Approach |
|---|---|
| Identity | Ed25519 keypair stored locally; private key never leaves the device |
| Invite tokens | Signed by the space host; single-use per generation; revocable |
| Token deduplication | SHA-256 hash as primary key prevents replay |
| No tracking | Zero telemetry, zero analytics, zero network calls at rest |

---

## Building & Development

```bash
# Run all tests
cargo test --workspace

# Check everything compiles
cargo check --workspace

# Run CLI in dev mode
cargo run -p monotask-cli -- ai-help

# Run desktop app in dev mode
cd crates/monotask-tauri
cargo tauri dev
```

### Cutting a release

Push a version tag and CI builds macOS arm64, Windows x64, and Linux x86_64 (musl), creates a GitHub release, and updates the Homebrew formula automatically.

```bash
git tag v0.4.0
git push origin v0.4.0
```

> Requires a `HOMEBREW_TAP_TOKEN` secret in the repo's GitHub Actions settings with write access to `nokhodian/homebrew-tap`.

---

<div align="center">

Made with ☕ and Rust · [MIT License](LICENSE)

<sub>The monkey approves of your task management.</sub>

</div>
