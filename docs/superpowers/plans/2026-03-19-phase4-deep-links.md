# Phase 4 Feature Additions: Deep Link URL Scheme (`kanban://`)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Register a `kanban://` custom URL scheme so external tools, scripts, and git hooks can deep-link into specific boards and cards in the running GUI. Add `app-cli open` commands for terminal use.

**Architecture:** Tauri's deep link plugin registers the scheme at install time on all three platforms. The handler resolves `<card_ref>` by parsing it as a card number first (using `card_number_index`), falling back to UUID. CLI `app-cli open` invokes the OS URL handler. Card creation commands include a `deep_link` field in `--json` output.

**Tech Stack:** Rust 2021, `tauri-plugin-deep-link`, `tauri-plugin-opener`, `clap` v4, SQLite `card_number_index`

**Depends on:** Phase 1 (`card_number_index` table and `resolve_card_ref` function).

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| Modify | `crates/kanban-tauri/src-tauri/Cargo.toml` | Add `tauri-plugin-deep-link` |
| Modify | `crates/kanban-tauri/src-tauri/tauri.conf.json` | Register `kanban` URL scheme |
| Create | `crates/kanban-tauri/src-tauri/src/deep_link.rs` | URL parsing, resolution, navigation dispatch |
| Modify | `crates/kanban-tauri/src-tauri/src/main.rs` | Register plugin + deep link handler |
| Modify | `crates/kanban-cli/src/commands/card.rs` | Add `deep_link` field to `--json` output of card creation |
| Modify | `crates/kanban-cli/src/commands/board.rs` | Add `deep_link` field to `--json` output of board creation |
| Create | `crates/kanban-cli/src/commands/open.rs` | `app-cli open board/card` |
| Modify | `crates/kanban-cli/src/main.rs` | Register `open` subcommand |

---

### Task 1: Register `kanban://` scheme in Tauri config

**Files:**
- Modify: `crates/kanban-tauri/src-tauri/Cargo.toml`
- Modify: `crates/kanban-tauri/src-tauri/tauri.conf.json`

- [ ] **Step 1: Add dependency**

In `crates/kanban-tauri/src-tauri/Cargo.toml`:
```toml
[dependencies]
tauri-plugin-deep-link = "2"
tauri-plugin-opener = "2"
```

- [ ] **Step 2: Register scheme in tauri.conf.json**

```json
{
  "plugins": {
    "deep-link": {
      "mobile": [],
      "desktop": {
        "schemes": ["kanban"]
      }
    }
  }
}
```

This generates the platform-specific registrations:
- macOS: `CFBundleURLSchemes` in `Info.plist`
- Linux: `MimeType=x-scheme-handler/kanban` in `.desktop` file
- Windows: registry key under `HKEY_CLASSES_ROOT\kanban`

- [ ] **Step 3: Verify scheme is registered after `cargo tauri build`**

```bash
cargo tauri build --target <your-platform>
# macOS: check .app/Contents/Info.plist contains "kanban"
# Linux: check .desktop file contains "x-scheme-handler/kanban"
```

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-tauri/src-tauri/Cargo.toml crates/kanban-tauri/src-tauri/tauri.conf.json
git commit -m "feat(tauri): register kanban:// URL scheme via tauri-plugin-deep-link"
```

---

### Task 2: Deep link URL parser

**Files:**
- Create: `crates/kanban-tauri/src-tauri/src/deep_link.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// crates/kanban-tauri/src-tauri/src/deep_link.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_board_url() {
        let target = parse_deep_link("kanban://board/my-board-id").unwrap();
        assert!(matches!(target, DeepLinkTarget::Board { board_id } if board_id == "my-board-id"));
    }

    #[test]
    fn parse_card_url_with_uuid() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let target = parse_deep_link(&format!("kanban://card/my-board/{uuid}")).unwrap();
        assert!(matches!(target, DeepLinkTarget::Card { board_id, card_ref }
            if board_id == "my-board" && card_ref == uuid));
    }

    #[test]
    fn parse_card_url_with_number() {
        let target = parse_deep_link("kanban://card/my-board/a7f3-42").unwrap();
        assert!(matches!(target, DeepLinkTarget::Card { card_ref, .. } if card_ref == "a7f3-42"));
    }

    #[test]
    fn reject_unknown_scheme() {
        assert!(parse_deep_link("https://example.com").is_err());
    }

    #[test]
    fn reject_malformed_url() {
        assert!(parse_deep_link("kanban://").is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-tauri deep_link
```
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
// crates/kanban-tauri/src-tauri/src/deep_link.rs

#[derive(Debug, Clone)]
pub enum DeepLinkTarget {
    Board { board_id: String },
    Card { board_id: String, card_ref: String }, // card_ref is number OR uuid
}

#[derive(Debug, thiserror::Error)]
pub enum DeepLinkError {
    #[error("unsupported scheme: {0}")]
    WrongScheme(String),
    #[error("malformed deep link URL: {0}")]
    Malformed(String),
}

pub fn parse_deep_link(url: &str) -> Result<DeepLinkTarget, DeepLinkError> {
    let url = url.trim();
    let rest = url.strip_prefix("kanban://")
        .ok_or_else(|| DeepLinkError::WrongScheme(url.to_string()))?;

    let parts: Vec<&str> = rest.splitn(3, '/').collect();
    match parts.as_slice() {
        ["board", board_id] if !board_id.is_empty() => {
            Ok(DeepLinkTarget::Board { board_id: board_id.to_string() })
        }
        ["card", board_id, card_ref] if !board_id.is_empty() && !card_ref.is_empty() => {
            Ok(DeepLinkTarget::Card {
                board_id: board_id.to_string(),
                card_ref: card_ref.to_string(),
            })
        }
        _ => Err(DeepLinkError::Malformed(url.to_string())),
    }
}

/// Resolve a `DeepLinkTarget::Card` to a concrete UUID, using card_number_index.
pub fn resolve_card_target(
    storage: &kanban_storage::Storage,
    board_id: &str,
    card_ref: &str,
) -> Result<String, crate::Error> {
    // Determine if card_ref matches the card number pattern (<4-8 chars>-<int>)
    if card_ref.parse::<kanban_core::card_number::CardNumber>().is_ok() {
        // Look up in card_number_index
        storage.resolve_card_ref(board_id, card_ref)
            .map_err(|e| crate::Error::DeepLink(e.to_string()))
    } else {
        // Treat as UUID — pass through
        Ok(card_ref.to_string())
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-tauri deep_link
```
Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-tauri/src-tauri/src/deep_link.rs
git commit -m "feat(tauri): deep link URL parser with card number resolution"
```

---

### Task 3: Wire deep link handler into Tauri app

**Files:**
- Modify: `crates/kanban-tauri/src-tauri/src/main.rs`

- [ ] **Step 1: Write the test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::deep_link::parse_deep_link;

    #[test]
    fn deep_link_handler_navigates_to_board() {
        // Use a mock AppHandle; verify that "navigate" event is emitted
        let result = handle_deep_link_url("kanban://board/test-board", &mock_state());
        assert!(result.is_ok());
    }

    #[test]
    fn deep_link_handler_navigates_to_card() {
        let result = handle_deep_link_url("kanban://card/test-board/a7f3-1", &mock_state());
        assert!(result.is_ok());
    }

    #[test]
    fn deep_link_handler_emits_not_found_for_missing_board() {
        let result = handle_deep_link_url("kanban://board/nonexistent-board", &mock_state());
        // Not an error — emits a "deep-link-not-found" event instead
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 2: Implement the handler**

```rust
// crates/kanban-tauri/src-tauri/src/main.rs (or deep_link.rs)

pub async fn handle_deep_link_url(url: &str, state: &AppState) -> Result<(), crate::Error> {
    let target = crate::deep_link::parse_deep_link(url)
        .map_err(|e| crate::Error::DeepLink(e.to_string()))?;

    match target {
        DeepLinkTarget::Board { board_id } => {
            // Check board exists locally
            if state.board_exists(&board_id) {
                state.app_handle.emit("navigate", NavigatePayload::Board { board_id })?;
            } else {
                state.app_handle.emit("deep-link-not-found", NotFoundPayload {
                    kind: "board".into(),
                    id: board_id,
                })?;
            }
        }
        DeepLinkTarget::Card { board_id, card_ref } => {
            let storage = state.storage.lock().await;
            match crate::deep_link::resolve_card_target(&storage, &board_id, &card_ref) {
                Ok(card_uuid) => {
                    state.app_handle.emit("navigate", NavigatePayload::Card {
                        board_id,
                        card_id: card_uuid,
                    })?;
                }
                Err(_) => {
                    state.app_handle.emit("deep-link-not-found", NotFoundPayload {
                        kind: "card".into(),
                        id: card_ref,
                    })?;
                }
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Register plugin and handler in builder**

```rust
// In main.rs tauri::Builder::default() chain:
.plugin(tauri_plugin_deep_link::init())
.setup(|app| {
    // Handle deep links delivered while app is running
    app.listen("deep-link://new-url", |event| {
        if let Some(url) = event.payload() {
            let _ = handle_deep_link_url(url, &app_state);
        }
    });
    // Handle deep link from launch args (app was not running)
    if let Some(url) = tauri_plugin_deep_link::get_current(app)? {
        handle_deep_link_url(&url, &app_state)?;
    }
    Ok(())
})
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-tauri deep_link_handler
```
Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-tauri/src-tauri/src/main.rs
git commit -m "feat(tauri): wire deep link handler for navigate and not-found events"
```

---

### Task 4: Add `deep_link` field to CLI `--json` output

**Files:**
- Modify: `crates/kanban-cli/src/commands/card.rs`
- Modify: `crates/kanban-cli/src/commands/board.rs`

Every create/copy mutation that outputs `--json` should include a `deep_link` field.

- [ ] **Step 1: Write the test**

```rust
#[test]
fn card_create_json_includes_deep_link() {
    let tmp = tempfile::tempdir().unwrap();
    let out = run_cli(&tmp, &["card", "create", board_id, col_id, "My Task", "--json"]);
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let deep_link = result["deep_link"].as_str().unwrap();
    assert!(deep_link.starts_with("kanban://card/"));
}

#[test]
fn board_create_json_includes_deep_link() {
    let tmp = tempfile::tempdir().unwrap();
    let out = run_cli(&tmp, &["board", "create", "My Board", "--json"]);
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let deep_link = result["deep_link"].as_str().unwrap();
    assert!(deep_link.starts_with("kanban://board/"));
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-cli deep_link_in_json
```
Expected: FAIL — `deep_link` field absent.

- [ ] **Step 3: Implement**

In `create_card` JSON serialization:
```rust
#[derive(serde::Serialize)]
struct CardCreateOutput {
    id: String,
    number: String,
    board_id: String,
    col_id: String,
    hlc: String,
    deep_link: String, // NEW
}

// When building the output:
let deep_link = format!("kanban://card/{}/{}", board_id, card.number.as_ref().map(|n| n.to_display()).unwrap_or(card.id.clone()));
```

In `create_board` JSON serialization:
```rust
let deep_link = format!("kanban://board/{}", board_id);
```

Also add to `copy_card` output.

- [ ] **Step 4: Run tests**

```bash
cargo test -p kanban-cli deep_link_in_json
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-cli/src/commands/card.rs crates/kanban-cli/src/commands/board.rs
git commit -m "feat(cli): add deep_link field to card/board --json output"
```

---

### Task 5: `app-cli open` command

**Files:**
- Create: `crates/kanban-cli/src/commands/open.rs`
- Modify: `crates/kanban-cli/src/main.rs`

- [ ] **Step 1: Write the test**

```rust
#[test]
fn open_board_produces_kanban_url() {
    // Mock the OS opener to capture the URL instead of opening it
    let url = build_open_url("board", "my-board-id", None);
    assert_eq!(url, "kanban://board/my-board-id");
}

#[test]
fn open_card_with_number() {
    let url = build_open_url("card", "my-board-id", Some("a7f3-42"));
    assert_eq!(url, "kanban://card/my-board-id/a7f3-42");
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kanban-cli open_board_produces
```
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
// crates/kanban-cli/src/commands/open.rs

use clap::Subcommand;

#[derive(Subcommand)]
pub enum OpenCommand {
    /// Open a board in the GUI
    Board {
        board_id: String,
    },
    /// Open a card in the GUI (card_ref = number or UUID)
    Card {
        board_id: String,
        card_ref: String,
    },
}

pub fn build_open_url(kind: &str, board_id: &str, card_ref: Option<&str>) -> String {
    match card_ref {
        None => format!("kanban://{kind}/{board_id}"),
        Some(r) => format!("kanban://{kind}/{board_id}/{r}"),
    }
}

pub fn run(cmd: OpenCommand) -> anyhow::Result<()> {
    let url = match &cmd {
        OpenCommand::Board { board_id } => build_open_url("board", board_id, None),
        OpenCommand::Card { board_id, card_ref } => build_open_url("card", board_id, Some(card_ref)),
    };

    // Attempt to open via OS URL handler
    // Falls back gracefully on headless systems
    match open::that(&url) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Warning: could not open URL handler (headless system?): {e}");
            eprintln!("Deep link: {url}");
        }
    }
    Ok(())
}
```

Add `open = "5"` to `crates/kanban-cli/Cargo.toml` dependencies for the `open` crate (cross-platform URL opener).

- [ ] **Step 4: Register in main.rs**

```rust
// In CLI app definition:
.subcommand(OpenCommand::augment_subcommands(Command::new("open")))
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p kanban-cli open
```
Expected: both tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/kanban-cli/src/commands/open.rs crates/kanban-cli/src/main.rs crates/kanban-cli/Cargo.toml
git commit -m "feat(cli): add app-cli open board/card command"
```

---

### Task 6: End-to-end deep link smoke test

- [ ] **Step 1: Write the test**

```rust
// tests/integration/phase4_smoke_test.rs

#[test]
fn deep_link_url_parses_correctly_from_card_create_output() {
    // Create a card with --json
    // Extract deep_link from output
    // Parse it with parse_deep_link
    // Verify it resolves to the correct card UUID via resolve_card_target
    let out = run_cli(&tmp, &["card", "create", board_id, col_id, "Smoke Test Card", "--json"]);
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let deep_link = result["deep_link"].as_str().unwrap();

    let target = kanban_tauri::deep_link::parse_deep_link(deep_link).unwrap();
    if let DeepLinkTarget::Card { board_id: b, card_ref } = target {
        let storage = Storage::open(tmp.path()).unwrap();
        let uuid = kanban_tauri::deep_link::resolve_card_target(&storage, &b, &card_ref).unwrap();
        assert_eq!(uuid, result["id"].as_str().unwrap());
    } else {
        panic!("expected card target");
    }
}

#[test]
fn open_command_produces_valid_url_without_crashing_on_headless() {
    // Run app-cli open board <id> and verify exit 0 even without a GUI
    let status = run_cli_status(&tmp, &["open", "board", "test-board-id"]);
    assert!(status.success());
}
```

- [ ] **Step 2: Run**

```bash
cargo test --test phase4_smoke_test
```
Expected: both tests pass.

- [ ] **Step 3: Final commit**

```bash
git add tests/integration/phase4_smoke_test.rs
git commit -m "test(integration): phase4 smoke tests for deep link parse, resolve, and open command"
```
