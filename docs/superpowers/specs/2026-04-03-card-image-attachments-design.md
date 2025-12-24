# Card Image Attachments — Design Spec

**Date:** 2026-04-03  
**Status:** Approved

## Summary

Cards can have images attached to them. Images are embedded in the card description using standard markdown image syntax (`![alt](img:id)`). Images sync to teammates via Automerge exactly like any other card field.

---

## 1. Data Model

Each card gains an `attachments` map in the Automerge doc alongside existing fields:

```
card {
  title: "Fix checkout bug"
  description: "Investigated the issue.\n![error screenshot](img:a1b2c3)\nFixed on line 42."
  attachments: {
    "a1b2c3": { name: "screenshot.png", mime: "image/png", data: "<base64>" }
    "d4e5f6": { name: "diagram.jpg",    mime: "image/jpeg", data: "<base64>" }
  }
}
```

- **Key:** 6-char lowercase hex id, generated at attach time (e.g. `a1b2c3`)
- **`data`:** raw base64-encoded bytes (no `data:...` URI prefix)
- **`name`:** original filename for display
- **`mime`:** MIME type for correct `data:` URI construction on render
- **Description:** plain markdown string; images referenced as `![alt](img:id)` tokens
- **Sync:** attachments travel with the card through Automerge automatically
- **Size guidance:** no hard limit enforced; soft recommendation ≤2MB per image to avoid bloating the Automerge doc

---

## 2. UI — Card Detail Panel

### Tab bar
A two-tab toggle appears above the description area:

```
[ Edit ]  [ Preview ]
```

- **Edit tab:** existing textarea (current behavior preserved)
- **Preview tab:** read-only rendered view of the description markdown

### Toolbar (Edit mode only)
A single row above the textarea with one button:

```
[ 📎 Image ]
```

Clicking **📎 Image** triggers a hidden `<input type="file" accept="image/*">`. On file select:
1. Read file as base64
2. Generate a 6-char hex id
3. Call Tauri `attach_image` command (persists immediately)
4. Insert `![filename](img:id)` at the current cursor position in the textarea

### Clipboard paste
The textarea listens for `paste` events. If `event.clipboardData` contains an image (`image/*`), it is handled identically to a file-picker selection — no extra UI needed.

### Preview rendering
A custom renderer (no external library) handles the description when Preview tab is active:

| Input | Output |
|---|---|
| `**bold**` | `<strong>bold</strong>` |
| `*italic*` | `<em>italic</em>` |
| `` `code` `` | `<code>code</code>` |
| `# Heading` | `<h1>–<h3>` |
| `- item` | `<ul><li>` |
| `![alt](img:id)` | `<img src="data:<mime>;base64,<data>" style="max-width:100%">` |
| Unknown `img:id` | `[image not found: id]` in muted text |

Image resolution: the renderer receives the card's `attachments` map and substitutes base64 data inline.

### Save behavior
Unchanged — description saved on blur. Attachments saved immediately on attach (separate Tauri command), ensuring they're persisted before the description is saved.

---

## 3. Rust Backend

### `monotask-core/src/card.rs`

New struct:
```rust
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data_b64: String,
}
```

New functions:
```rust
pub fn attach_image(
    doc: &mut AutoCommit,
    card_id: &str,
    id: &str,
    name: &str,
    mime: &str,
    data_b64: &str,
) -> Result<()>

pub fn remove_attachment(
    doc: &mut AutoCommit,
    card_id: &str,
    attachment_id: &str,
) -> Result<()>
```

`read_card` gains:
```rust
pub attachments: HashMap<String, Attachment>
```

### `monotask-tauri/src-tauri/src/main.rs`

Two new Tauri commands:
```rust
#[tauri::command]
fn attach_image(board_id: String, card_id: String, name: String, mime: String, data_b64: String)
    -> Result<String, String>
// Returns the generated 6-char hex id

#[tauri::command]
fn remove_attachment(board_id: String, card_id: String, attachment_id: String)
    -> Result<(), String>
```

---

## 4. CLI

New subcommand:
```
monotask card attach-image <board-id> <card-id> <file-path> [--json]
```

Behavior:
1. Read file from `<file-path>`
2. Base64-encode the contents
3. Detect MIME type from file extension
4. Store via `attach_image` core function
5. Print the generated `img:id` token (or JSON with `{id, name, mime}`)

`card view --json` output gains an `attachments` field:
```json
"attachments": [
  { "id": "a1b2c3", "name": "screenshot.png", "mime": "image/png" }
]
```
Base64 data is excluded from `card view` output (too verbose). A separate `card get-attachment <board-id> <card-id> <id>` command is out of scope for this iteration.

---

## 5. Out of Scope

- Image deletion UI (can be done later; the core `remove_attachment` function will exist)
- Drag-and-drop onto the editor
- Image resizing or captions
- `card get-attachment` CLI command
- Any size enforcement or compression
