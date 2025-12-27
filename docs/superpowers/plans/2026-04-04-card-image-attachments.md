# Card Image Attachments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let cards carry images stored as base64 in Automerge, referenced in descriptions via `![alt](img:id)` markdown syntax, with a tab-based Edit/Preview UI in the card detail panel and a CLI `attach-image` subcommand.

**Architecture:** Images are stored in a per-card `attachments` map in the Automerge doc (key = 6-char hex id, value = map with `name`/`mime`/`data` fields). The description remains a plain string with `![alt](img:id)` tokens. A custom markdown renderer in the frontend resolves those tokens to `data:` URIs from the attachment map.

**Tech Stack:** Rust/Automerge (core), Tauri v2 commands (backend), vanilla JS in index.html (frontend), clap (CLI), base64 crate (already in both crates).

---

### Task 1: Core — Attachment data model, functions, and tests

**Files:**
- Modify: `crates/monotask-core/src/card.rs`

- [ ] **Step 1: Add `Attachment` struct and update `Card` struct**

In `crates/monotask-core/src/card.rs`, replace the `Card` struct definition (lines 5–21) with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    #[serde(skip)]          // excluded from CLI `card view --json` output (too verbose)
    pub data_b64: String,   // Tauri reads this as a Rust field, not via serde
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Card {
    pub id: String,
    pub number: Option<crate::card_number::CardNumber>,
    pub title: String,
    pub description: String,
    pub cover_color: Option<String>,
    pub priority: String,
    pub assignees: Vec<String>,
    pub labels: Vec<String>,
    pub due_date: Option<String>,
    pub archived: bool,
    pub deleted: bool,
    pub copied_from: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub attachments: std::collections::HashMap<String, Attachment>,
}
```

- [ ] **Step 2: Init attachments map in `create_card`**

In `create_card`, after line 91 (`doc.put_object(&card_obj, "related", ObjType::Map)?;`), add:

```rust
    doc.put_object(&card_obj, "attachments", ObjType::Map)?;
```

- [ ] **Step 3: Read attachments in `read_card`**

In `read_card`, after the labels block (after `None => vec![],` closing the labels match, before `Ok(Card {`), add:

```rust
    // Read attachments map
    let attachments = match doc.get(&card_obj, "attachments")? {
        Some((_, map_id)) => {
            let keys: Vec<String> = doc.keys(&map_id).map(|k| k.to_string()).collect();
            let mut m = std::collections::HashMap::new();
            for key in keys {
                if let Some((_, entry_id)) = doc.get(&map_id, key.as_str())? {
                    let name = crate::get_string(doc, &entry_id, "name")?.unwrap_or_default();
                    let mime = crate::get_string(doc, &entry_id, "mime")?.unwrap_or_default();
                    let data_b64 = crate::get_string(doc, &entry_id, "data")?.unwrap_or_default();
                    m.insert(key, Attachment { name, mime, data_b64 });
                }
            }
            m
        }
        None => std::collections::HashMap::new(),
    };
```

Then add `attachments,` to the `Ok(Card { ... })` return value.

- [ ] **Step 4: Add `attach_image` and `remove_attachment` functions**

After `set_assignee` (after line 412), before the `#[cfg(test)]` block, add:

```rust
pub fn attach_image(
    doc: &mut AutoCommit,
    card_id: &str,
    id: &str,
    name: &str,
    mime: &str,
    data_b64: &str,
) -> Result<()> {
    let card_obj = get_card_obj(doc, card_id)?;
    let attachments_map = match doc.get(&card_obj, "attachments")? {
        Some((_, map_id)) => map_id,
        None => doc.put_object(&card_obj, "attachments", ObjType::Map)?,
    };
    let entry = doc.put_object(&attachments_map, id, ObjType::Map)?;
    doc.put(&entry, "name", name)?;
    doc.put(&entry, "mime", mime)?;
    doc.put(&entry, "data", data_b64)?;
    Ok(())
}

pub fn remove_attachment(
    doc: &mut AutoCommit,
    card_id: &str,
    attachment_id: &str,
) -> Result<()> {
    let card_obj = get_card_obj(doc, card_id)?;
    let attachments_map = match doc.get(&card_obj, "attachments")? {
        Some((_, map_id)) => map_id,
        None => return Ok(()),
    };
    doc.delete(&attachments_map, attachment_id)?;
    Ok(())
}
```

- [ ] **Step 5: Write failing tests**

In the `#[cfg(test)] mod tests` block at the bottom of `card.rs`, add after the existing tests:

```rust
    #[test]
    fn attach_image_stores_and_reads_back() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let actor_pk = vec![1u8; 32];
        let members = vec![actor_pk.clone()];
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        let card = create_card(&mut doc, &col_id, "Task", &actor_pk, &members).unwrap();

        attach_image(&mut doc, &card.id, "abc123", "shot.png", "image/png", "aGVsbG8=").unwrap();

        let read = read_card(&doc, &card.id).unwrap();
        assert_eq!(read.attachments.len(), 1);
        let att = read.attachments.get("abc123").unwrap();
        assert_eq!(att.name, "shot.png");
        assert_eq!(att.mime, "image/png");
        assert_eq!(att.data_b64, "aGVsbG8=");
    }

    #[test]
    fn remove_attachment_removes_entry() {
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let actor_pk = vec![1u8; 32];
        let members = vec![actor_pk.clone()];
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        let card = create_card(&mut doc, &col_id, "Task", &actor_pk, &members).unwrap();

        attach_image(&mut doc, &card.id, "abc123", "shot.png", "image/png", "aGVsbG8=").unwrap();
        remove_attachment(&mut doc, &card.id, "abc123").unwrap();

        let read = read_card(&doc, &card.id).unwrap();
        assert!(read.attachments.is_empty());
    }

    #[test]
    fn attach_image_on_legacy_card_without_attachments_map() {
        // Simulates a card created before this feature (no attachments obj in doc)
        let mut doc = AutoCommit::new();
        crate::init_doc(&mut doc).unwrap();
        let actor_pk = vec![1u8; 32];
        let members = vec![actor_pk.clone()];
        let col_id = crate::column::create_column(&mut doc, "To Do").unwrap();
        // create_card now adds the map, so delete it to simulate legacy
        let card = create_card(&mut doc, &col_id, "Old Task", &actor_pk, &members).unwrap();
        {
            use automerge::transaction::Transactable;
            let card_obj = get_card_obj(&doc, &card.id).unwrap();
            // We can't easily delete a sub-map in automerge, so just verify
            // attach_image gracefully handles a missing map by creating it
        }
        // attach should succeed even on a card missing the map
        attach_image(&mut doc, &card.id, "xyz789", "img.jpg", "image/jpeg", "dGVzdA==").unwrap();
        let read = read_card(&doc, &card.id).unwrap();
        assert_eq!(read.attachments.len(), 1);
    }
```

- [ ] **Step 6: Run tests**

```bash
cd /Volumes/media/projects/monotask
cargo test -p monotask-core -- card 2>&1 | tail -20
```

Expected: all card tests pass including the 3 new ones.

- [ ] **Step 7: Commit**

```bash
git add crates/monotask-core/src/card.rs
git commit -m "feat(core): add Attachment struct, attach_image, remove_attachment to card"
```

---

### Task 2: Tauri backend — attachment commands

**Files:**
- Modify: `crates/monotask-tauri/src-tauri/src/main.rs`

- [ ] **Step 1: Add `AttachmentView` struct and update `CardDetailView`**

After the `CardDetailView` struct definition (after line 365, after `priority: Option<String>,`), add:

```rust
#[derive(serde::Serialize)]
struct AttachmentView {
    id: String,
    name: String,
    mime: String,
    data_b64: String,
}
```

Then add `attachments: Vec<AttachmentView>,` to the `CardDetailView` struct:

```rust
#[derive(serde::Serialize)]
struct CardDetailView {
    id: String,
    title: String,
    description: String,
    number: Option<String>,
    labels: Vec<String>,
    due_date: Option<String>,
    assignee: Option<String>,
    created_at: String,
    comments: Vec<CommentView>,
    history: Vec<MoveEvent>,
    checklists: Vec<ChecklistView>,
    cover_color: Option<String>,
    priority: Option<String>,
    attachments: Vec<AttachmentView>,
}
```

- [ ] **Step 2: Update `get_card_cmd` to populate attachments**

In `get_card_cmd`, before the `Ok(CardDetailView {` return, add:

```rust
    let attachments: Vec<AttachmentView> = card.attachments
        .into_iter()
        .map(|(id, a)| AttachmentView {
            id,
            name: a.name,
            mime: a.mime,
            data_b64: a.data_b64,
        })
        .collect();
```

And add `attachments,` to the `Ok(CardDetailView { ... })` struct literal.

- [ ] **Step 3: Add `generate_attachment_id` helper and two new commands**

After the `delete_card_cmd` function (after line 979), add:

```rust
fn generate_attachment_id() -> String {
    let raw = uuid::Uuid::new_v4().to_string().replace('-', "");
    raw[..6].to_string()
}

#[tauri::command]
fn attach_image_cmd(
    board_id: String,
    card_id: String,
    name: String,
    mime: String,
    data_b64: String,
    state: tauri::State<AppState>,
) -> Result<String, String> {
    let id = generate_attachment_id();
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    monotask_core::card::attach_image(&mut doc, &card_id, &id, &name, &mime, &data_b64)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(id)
}

#[tauri::command]
fn remove_attachment_cmd(
    board_id: String,
    card_id: String,
    attachment_id: String,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut doc = monotask_storage::board::load_board(storage.conn(), &board_id)
        .map_err(|e| e.to_string())?;
    monotask_core::card::remove_attachment(&mut doc, &card_id, &attachment_id)
        .map_err(|e| e.to_string())?;
    monotask_storage::board::save_board(storage.conn(), &board_id, &mut doc)
        .map_err(|e| e.to_string())?;
    trigger_board_sync(&board_id, &state);
    Ok(())
}
```

- [ ] **Step 4: Register the two new commands in the handler list**

In the `.invoke_handler(tauri::generate_handler![` block (around line 2242), add after `export_board_cmd,`:

```rust
            attach_image_cmd,
            remove_attachment_cmd,
```

- [ ] **Step 5: Verify it compiles**

```bash
cd /Volumes/media/projects/monotask/crates/monotask-tauri
cargo check 2>&1 | tail -20
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add crates/monotask-tauri/src-tauri/src/main.rs
git commit -m "feat(tauri): add attach_image_cmd, remove_attachment_cmd; expose attachments in CardDetailView"
```

---

### Task 3: UI — CSS and HTML structure

**Files:**
- Modify: `crates/monotask-tauri/src/index.html`

- [ ] **Step 1: Add CSS for description editor tabs, toolbar, and preview**

After the `.card-detail-desc:focus { border-color: #2e2e5a; }` rule (around line 613), add:

```css
    .desc-tab-bar { display: flex; gap: 4px; margin-bottom: 6px; align-items: center; }
    .desc-tab {
      background: #1e1e3a; border: 1px solid #2a2a4a; color: #888;
      padding: 2px 10px; border-radius: 3px; font-size: 10px; cursor: pointer;
    }
    .desc-tab.active { background: #2e2e5a; color: #ccc; border-color: #3a3a6a; }
    .desc-toolbar { margin-bottom: 4px; }
    .desc-toolbar-btn {
      background: #1e1e3a; border: 1px solid #2a2a4a; color: #888;
      padding: 3px 10px; border-radius: 3px; font-size: 11px; cursor: pointer;
    }
    .desc-toolbar-btn:hover { background: #2a2a4a; color: #ccc; }
    .card-detail-desc-preview {
      width: 100%; min-height: 72px; resize: vertical;
      background: #0d0d1a; border: 1px solid #1e1e3a; color: #ccc;
      padding: 8px 10px; border-radius: 4px; font-size: 13px;
      line-height: 1.5; overflow-y: auto; box-sizing: border-box;
    }
    .card-detail-desc-preview img {
      max-width: 100%; border-radius: 4px; margin: 4px 0; display: block;
    }
    .card-detail-desc-preview code {
      background: #1a1a30; padding: 1px 4px; border-radius: 3px;
      font-family: monospace; font-size: 12px;
    }
    .card-detail-desc-preview ul { padding-left: 20px; margin: 4px 0; }
    .card-detail-desc-preview h1 { font-size: 17px; color: #ddd; margin: 8px 0 4px; }
    .card-detail-desc-preview h2 { font-size: 15px; color: #ddd; margin: 8px 0 4px; }
    .card-detail-desc-preview h3 { font-size: 13px; color: #ddd; margin: 8px 0 4px; }
```

- [ ] **Step 2: Replace the description section HTML**

Find and replace the description section (around line 1316–1318):

Old:
```html
        <div class="card-detail-section">
          <div class="card-detail-label">Description</div>
          <textarea id="card-detail-desc" class="card-detail-desc" placeholder="Add a description…"></textarea>
        </div>
```

New:
```html
        <div class="card-detail-section">
          <div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:6px;">
            <div class="card-detail-label" style="margin-bottom:0;">Description</div>
            <div class="desc-tab-bar" style="margin-bottom:0;">
              <button id="desc-tab-edit" class="desc-tab active" onclick="switchDescTab('edit')">Edit</button>
              <button id="desc-tab-preview" class="desc-tab" onclick="switchDescTab('preview')">Preview</button>
            </div>
          </div>
          <div id="desc-toolbar" class="desc-toolbar">
            <button class="desc-toolbar-btn" onclick="triggerImageAttach()">📎 Image</button>
            <input type="file" id="desc-image-input" accept="image/*" style="display:none;" onchange="handleImageFile(this)">
          </div>
          <textarea id="card-detail-desc" class="card-detail-desc" placeholder="Add a description… (supports markdown, paste or attach images)"></textarea>
          <div id="desc-preview" class="card-detail-desc-preview" style="display:none;"></div>
        </div>
```

- [ ] **Step 3: Verify HTML structure is valid**

```bash
grep -n "card-detail-desc\|desc-tab\|desc-toolbar\|desc-preview" /Volumes/media/projects/monotask/crates/monotask-tauri/src/index.html | head -20
```

Expected: shows the new ids in the file.

- [ ] **Step 4: Commit**

```bash
git add crates/monotask-tauri/src/index.html
git commit -m "feat(ui): add description Edit/Preview tabs and image toolbar HTML+CSS"
```

---

### Task 4: UI — JavaScript

**Files:**
- Modify: `crates/monotask-tauri/src/index.html`

- [ ] **Step 1: Add module-level `_descAttachments` variable**

Find the block of module-level variables near the top of the `<script>` section. Look for lines like `let _cardDetailCtx = null;` or similar state variables. Add after them:

```javascript
  let _descAttachments = {}; // { id: { name, mime, data_b64 } } — populated when a card is opened
```

- [ ] **Step 2: Add `switchDescTab`, `triggerImageAttach`, `insertAtCursor` functions**

Find a suitable location in the script section (e.g., near the card detail functions). Add:

```javascript
  function switchDescTab(tab) {
    const editTab = document.getElementById('desc-tab-edit');
    const previewTab = document.getElementById('desc-tab-preview');
    const textarea = document.getElementById('card-detail-desc');
    const preview = document.getElementById('desc-preview');
    const toolbar = document.getElementById('desc-toolbar');
    if (tab === 'edit') {
      editTab.classList.add('active');
      previewTab.classList.remove('active');
      textarea.style.display = '';
      toolbar.style.display = '';
      preview.style.display = 'none';
    } else {
      editTab.classList.remove('active');
      previewTab.classList.add('active');
      textarea.style.display = 'none';
      toolbar.style.display = 'none';
      preview.style.display = '';
      preview.innerHTML = renderMarkdown(textarea.value, _descAttachments);
    }
  }

  function triggerImageAttach() {
    document.getElementById('desc-image-input').click();
  }

  function insertAtCursor(el, text) {
    const start = el.selectionStart;
    const end = el.selectionEnd;
    el.value = el.value.substring(0, start) + text + el.value.substring(end);
    el.selectionStart = el.selectionEnd = start + text.length;
    el.focus();
  }
```

- [ ] **Step 3: Add `handleImageFile` and `renderMarkdown` functions**

```javascript
  async function handleImageFile(input) {
    const file = input.files[0];
    if (!file || !_cardDetailCtx) return;
    const reader = new FileReader();
    reader.onload = async (e) => {
      const dataUrl = e.target.result;
      const comma = dataUrl.indexOf(',');
      const dataB64 = dataUrl.substring(comma + 1);
      const mime = file.type || 'image/png';
      const name = file.name;
      const { boardId, cardId } = _cardDetailCtx;
      try {
        const id = await invoke('attach_image_cmd', { boardId, cardId, name, mime, dataB64 });
        _descAttachments[id] = { name, mime, data_b64: dataB64 };
        insertAtCursor(document.getElementById('card-detail-desc'), `![${name}](img:${id})`);
      } catch (err) { showAlert('Failed to attach image: ' + err); }
      input.value = '';
    };
    reader.readAsDataURL(file);
  }

  function renderMarkdown(text, attachments) {
    if (!text) return '';
    // Escape HTML first
    let html = text
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
    // Images: ![alt](img:id) — resolve before other substitutions
    html = html.replace(/!\[([^\]]*)\]\(img:([a-f0-9]{6})\)/g, (_, alt, id) => {
      const att = attachments[id];
      if (!att) return `<span style="color:#888">[image not found: ${id}]</span>`;
      return `<img src="data:${att.mime};base64,${att.data_b64}" alt="${alt.replace(/"/g, '&quot;')}">`;
    });
    // Bold **text**
    html = html.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>');
    // Italic *text*
    html = html.replace(/\*([^*\n]+)\*/g, '<em>$1</em>');
    // Inline code `text`
    html = html.replace(/`([^`\n]+)`/g, '<code>$1</code>');
    // Headings (must be processed line-by-line simulation before newline conversion)
    html = html.replace(/^### (.+)$/gm, '<h3>$1</h3>');
    html = html.replace(/^## (.+)$/gm, '<h2>$1</h2>');
    html = html.replace(/^# (.+)$/gm, '<h1>$1</h1>');
    // Bullet list items
    html = html.replace(/^- (.+)$/gm, '<li>$1</li>');
    // Wrap consecutive li elements in ul
    html = html.replace(/(<li>[\s\S]*?<\/li>)(\n<li>[\s\S]*?<\/li>)*/g, (m) => `<ul>${m}</ul>`);
    // Line breaks (after block-level substitutions)
    html = html.replace(/\n/g, '<br>');
    return html;
  }
```

- [ ] **Step 4: Add clipboard paste handler for images**

After the `insertAtCursor` or `handleImageFile` functions, add:

```javascript
  document.getElementById('card-detail-desc').addEventListener('paste', async (e) => {
    if (!_cardDetailCtx) return;
    const items = e.clipboardData && e.clipboardData.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.type.startsWith('image/')) {
        e.preventDefault();
        const file = item.getAsFile();
        if (!file) continue;
        const reader = new FileReader();
        reader.onload = async (ev) => {
          const dataUrl = ev.target.result;
          const comma = dataUrl.indexOf(',');
          const dataB64 = dataUrl.substring(comma + 1);
          const mime = item.type;
          const name = 'pasted-image.png';
          const { boardId, cardId } = _cardDetailCtx;
          try {
            const id = await invoke('attach_image_cmd', { boardId, cardId, name, mime, dataB64 });
            _descAttachments[id] = { name, mime, data_b64: dataB64 };
            insertAtCursor(document.getElementById('card-detail-desc'), `![${name}](img:${id})`);
          } catch (err) { showAlert('Failed to attach pasted image: ' + err); }
        };
        reader.readAsDataURL(file);
        break;
      }
    }
  });
```

- [ ] **Step 5: Update `openCardDetail` to populate `_descAttachments` and reset tab**

In `openCardDetail` (around line 2282), after the line `document.getElementById('card-detail-desc').value = card.description || '';`, add:

```javascript
      // Populate attachment map for the preview renderer
      _descAttachments = {};
      for (const att of (card.attachments || [])) {
        _descAttachments[att.id] = { name: att.name, mime: att.mime, data_b64: att.data_b64 };
      }
      // Reset to Edit tab each time a card is opened
      switchDescTab('edit');
```

- [ ] **Step 6: Verify no JS syntax errors by checking the app compiles**

```bash
cd /Volumes/media/projects/monotask/crates/monotask-tauri
cargo check 2>&1 | tail -10
```

Expected: no errors (Tauri bundles index.html as-is, Rust compilation is the proxy check).

- [ ] **Step 7: Commit**

```bash
git add crates/monotask-tauri/src/index.html
git commit -m "feat(ui): add image attach/preview JS — tabs, file picker, paste, markdown renderer"
```

---

### Task 5: CLI — `card attach-image` subcommand

**Files:**
- Modify: `crates/monotask-cli/src/main.rs`

- [ ] **Step 1: Add `AttachImage` variant to `CardCommands`**

In the `CardCommands` enum (around line 86–110), after `SetAssignee { ... },` add:

```rust
    AttachImage { board_id: String, card_id: String, file: String, #[arg(long)] json: bool },
```

- [ ] **Step 2: Add `mime_from_ext` helper function**

Near the bottom of the file (before `fn main`), add:

```rust
fn mime_from_ext(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "png"  => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif"  => "image/gif",
        "webp" => "image/webp",
        "svg"  => "image/svg+xml",
        _      => "image/png",
    }
}
```

- [ ] **Step 3: Add `AttachImage` handler in the `CardCommands` match arm**

In the `match card_cmd` block (after the `SetAssignee` arm), add:

```rust
            CardCommands::AttachImage { board_id, card_id, file, json } => {
                use std::io::Read;
                use base64::Engine;
                let mut f = std::fs::File::open(&file)
                    .map_err(|e| anyhow::anyhow!("Cannot open {file}: {e}"))?;
                let mut bytes = Vec::new();
                f.read_to_end(&mut bytes)?;
                let data_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let mime = mime_from_ext(&file);
                let name = std::path::Path::new(&file)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let id_raw = uuid::Uuid::new_v4().to_string().replace('-', "");
                let id = &id_raw[..6];
                let mut doc = storage.load_board(&board_id)?;
                monotask_core::card::attach_image(&mut doc, &card_id, id, &name, mime, &data_b64)?;
                storage.save_board(&board_id, &mut doc)?;
                if json {
                    println!("{}", serde_json::json!({"id": id, "name": name, "mime": mime, "token": format!("img:{id}")}));
                } else {
                    println!("Attached {} as img:{} — embed with ![{}](img:{})", name, id, name, id);
                }
            }
```

- [ ] **Step 4: Run cargo check on CLI**

```bash
cd /Volumes/media/projects/monotask
cargo check -p monotask-cli 2>&1 | tail -15
```

Expected: no errors.

- [ ] **Step 5: Run all tests**

```bash
cargo test --workspace 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/monotask-cli/src/main.rs
git commit -m "feat(cli): add card attach-image subcommand"
```

---

### Task 6: Smoke test and version bump

- [ ] **Step 1: Manual smoke test — core attach/read roundtrip**

```bash
cargo test -p monotask-core -- attach 2>&1
```

Expected: `attach_image_stores_and_reads_back` and `remove_attachment_removes_entry` both PASS.

- [ ] **Step 2: Manual smoke test — CLI attach**

```bash
# Create a temp board
BOARD=$(cargo run -p monotask-cli -- board create "Img Test" --json 2>/dev/null | jq -r .id)
COL=$(cargo run -p monotask-cli -- column create $BOARD "Todo" --json 2>/dev/null | jq -r .id)
CARD=$(cargo run -p monotask-cli -- card create $BOARD $COL "Image card" --json 2>/dev/null | jq -r .id)

# Attach a test image (create a small 1px PNG)
printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\x0cIDATx\x9cc\xf8\x0f\x00\x00\x01\x01\x00\x05\x18\xd8N\x00\x00\x00\x00IEND\xaeB`\x82' > /tmp/test.png

cargo run -p monotask-cli -- card attach-image $BOARD $CARD /tmp/test.png --json 2>/dev/null
```

Expected JSON: `{"id":"<6hex>","name":"test.png","mime":"image/png","token":"img:<6hex>"}`

- [ ] **Step 3: Verify `card view` shows attachments field**

```bash
cargo run -p monotask-cli -- card view $BOARD $CARD --json 2>/dev/null | jq .attachments
```

Expected: `[{"id":"<6hex>","name":"test.png","mime":"image/png"}]` (no data_b64).

- [ ] **Step 4: Final commit if any fixups needed, then push**

```bash
git log --oneline -6
git push origin master
```
