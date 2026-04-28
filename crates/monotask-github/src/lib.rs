use automerge::{AutoCommit, ReadDoc, ScalarValue, transaction::Transactable, ROOT};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use base64::Engine as _;

// ── Token management ──────────────────────────────────────────────────────────

pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join("github_token")
}

pub fn save_token(data_dir: &Path, token: &str) -> Result<()> {
    let path = token_path(data_dir);
    std::fs::write(&path, token.trim())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn load_token(data_dir: &Path) -> Result<Option<String>> {
    let path = token_path(data_dir);
    if !path.exists() {
        return Ok(None);
    }
    let t = std::fs::read_to_string(&path)?;
    let t = t.trim().to_string();
    Ok(if t.is_empty() { None } else { Some(t) })
}

pub fn delete_token(data_dir: &Path) -> Result<()> {
    let path = token_path(data_dir);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub async fn test_token(token: &str) -> Result<bool> {
    let client = GitHubClient::new("", "", token);
    client.test_token().await
}

// ── Board config in CRDT ──────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitHubConfig {
    pub owner: String,
    pub repo: String,
    pub done_column_id: String,
    pub last_sync: Option<String>,
}

pub fn get_github_config(doc: &AutoCommit) -> Option<GitHubConfig> {
    let owner = monotask_core::get_string(doc, &ROOT, "github_owner").ok()??;
    let repo = monotask_core::get_string(doc, &ROOT, "github_repo").ok()??;
    let done_col = monotask_core::get_string(doc, &ROOT, "github_done_column_id").ok()??;
    if owner.is_empty() || repo.is_empty() || done_col.is_empty() {
        return None;
    }
    let last_sync = monotask_core::get_string(doc, &ROOT, "github_last_sync")
        .ok().flatten()
        .and_then(|s| if s.is_empty() { None } else { Some(s) });
    Some(GitHubConfig { owner, repo, done_column_id: done_col, last_sync })
}

pub fn set_github_config(doc: &mut AutoCommit, config: Option<&GitHubConfig>) -> monotask_core::Result<()> {
    match config {
        Some(c) => {
            doc.put(ROOT, "github_owner", c.owner.as_str())?;
            doc.put(ROOT, "github_repo", c.repo.as_str())?;
            doc.put(ROOT, "github_done_column_id", c.done_column_id.as_str())?;
            doc.put(ROOT, "github_last_sync", c.last_sync.as_deref().unwrap_or(""))?;
        }
        None => {
            doc.put(ROOT, "github_owner", "")?;
            doc.put(ROOT, "github_repo", "")?;
            doc.put(ROOT, "github_done_column_id", "")?;
            doc.put(ROOT, "github_last_sync", "")?;
        }
    }
    Ok(())
}

// ── Card github fields in CRDT ────────────────────────────────────────────────

pub fn get_github_issue_number(doc: &AutoCommit, card_id: &str) -> Option<u64> {
    let card_obj = monotask_core::card::get_card_obj(doc, card_id).ok()?;
    match doc.get(&card_obj, "github_issue_number").ok()? {
        Some((automerge::Value::Scalar(s), _)) => match s.as_ref() {
            ScalarValue::Uint(n) => Some(*n),
            ScalarValue::Int(n) if *n > 0 => Some(*n as u64),
            _ => None,
        },
        _ => None,
    }
}

pub fn set_github_issue_number(doc: &mut AutoCommit, card_id: &str, number: u64) -> monotask_core::Result<()> {
    let cards_map = monotask_core::get_cards_map(doc)?;
    let card_obj = match doc.get(&cards_map, card_id)? {
        Some((_, id)) => id,
        None => return Err(monotask_core::Error::NotFound(card_id.into())),
    };
    doc.put(&card_obj, "github_issue_number", number)?;
    Ok(())
}

pub fn get_github_synced_at(doc: &AutoCommit, card_id: &str) -> Option<String> {
    let card_obj = monotask_core::card::get_card_obj(doc, card_id).ok()?;
    monotask_core::get_string(doc, &card_obj, "github_synced_at")
        .ok().flatten()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
}

pub fn set_github_synced_at(doc: &mut AutoCommit, card_id: &str, ts: &str) -> monotask_core::Result<()> {
    let cards_map = monotask_core::get_cards_map(doc)?;
    let card_obj = match doc.get(&cards_map, card_id)? {
        Some((_, id)) => id,
        None => return Err(monotask_core::Error::NotFound(card_id.into())),
    };
    doc.put(&card_obj, "github_synced_at", ts)?;
    Ok(())
}

// ── Comment github_id tracking in CRDT ───────────────────────────────────────

/// Returns map of github_comment_id → local comment id for a card's existing comments.
fn card_github_comment_ids(doc: &AutoCommit, card_id: &str) -> HashMap<u64, String> {
    let mut map = HashMap::new();
    let card_obj = match monotask_core::card::get_card_obj(doc, card_id) {
        Ok(o) => o,
        Err(_) => return map,
    };
    let comments_list = match monotask_core::comment::get_comments_list(doc, &card_obj) {
        Ok(l) => l,
        Err(_) => return map,
    };
    for i in 0..doc.length(&comments_list) {
        if let Ok(Some((_, c_obj))) = doc.get(&comments_list, i) {
            let deleted = matches!(
                doc.get(&c_obj, "deleted"),
                Ok(Some((automerge::Value::Scalar(s), _))) if matches!(s.as_ref(), ScalarValue::Boolean(true))
            );
            if deleted { continue; }
            let gh_id = match doc.get(&c_obj, "github_comment_id") {
                Ok(Some((automerge::Value::Scalar(s), _))) => match s.as_ref() {
                    ScalarValue::Uint(n) => *n,
                    ScalarValue::Int(n) if *n > 0 => *n as u64,
                    _ => continue,
                },
                _ => continue,
            };
            let local_id = match monotask_core::get_string(doc, &c_obj, "id") {
                Ok(Some(s)) => s,
                _ => continue,
            };
            map.insert(gh_id, local_id);
        }
    }
    map
}

/// Returns map of local comment id → github_comment_id for comments that have been pushed.
fn card_local_comment_gh_ids(doc: &AutoCommit, card_id: &str) -> HashMap<String, u64> {
    let mut map = HashMap::new();
    let card_obj = match monotask_core::card::get_card_obj(doc, card_id) {
        Ok(o) => o,
        Err(_) => return map,
    };
    let comments_list = match monotask_core::comment::get_comments_list(doc, &card_obj) {
        Ok(l) => l,
        Err(_) => return map,
    };
    for i in 0..doc.length(&comments_list) {
        if let Ok(Some((_, c_obj))) = doc.get(&comments_list, i) {
            let deleted = matches!(
                doc.get(&c_obj, "deleted"),
                Ok(Some((automerge::Value::Scalar(s), _))) if matches!(s.as_ref(), ScalarValue::Boolean(true))
            );
            if deleted { continue; }
            let gh_id = match doc.get(&c_obj, "github_comment_id") {
                Ok(Some((automerge::Value::Scalar(s), _))) => match s.as_ref() {
                    ScalarValue::Uint(n) => *n,
                    ScalarValue::Int(n) if *n > 0 => *n as u64,
                    _ => continue,
                },
                _ => continue,
            };
            let local_id = match monotask_core::get_string(doc, &c_obj, "id") {
                Ok(Some(s)) => s,
                _ => continue,
            };
            map.insert(local_id, gh_id);
        }
    }
    map
}

/// Tag the comment with the given local_comment_id with its GitHub comment id.
fn set_comment_github_id(doc: &mut AutoCommit, card_id: &str, local_comment_id: &str, gh_id: u64) {
    let card_obj = match monotask_core::card::get_card_obj(doc, card_id) {
        Ok(o) => o,
        Err(_) => return,
    };
    let comments_list = match monotask_core::comment::get_comments_list(doc, &card_obj) {
        Ok(l) => l,
        Err(_) => return,
    };
    for i in 0..doc.length(&comments_list) {
        if let Ok(Some((_, c_obj))) = doc.get(&comments_list, i) {
            if let Ok(Some(id)) = monotask_core::get_string(doc, &c_obj, "id") {
                if id == local_comment_id {
                    let _ = doc.put(&c_obj, "github_comment_id", gh_id);
                    return;
                }
            }
        }
    }
}

// ── GitHub API ────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Debug)]
struct Issue {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    labels: Vec<GhLabel>,
    updated_at: String,
    pull_request: Option<serde_json::Value>,
    #[serde(default)]
    comments: u64,
}

#[derive(serde::Deserialize, Debug)]
struct GhLabel {
    name: String,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct GhComment {
    id: u64,
    body: Option<String>,
    user: Option<GhUser>,
    #[allow(dead_code)]
    created_at: String,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct GhUser {
    login: String,
    avatar_url: String,
}

pub struct GitHubClient {
    owner: String,
    repo: String,
    token: String,
    client: reqwest::Client,
}

impl GitHubClient {
    pub fn new(owner: &str, repo: &str, token: &str) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("monotask-github/0.4")
            .build()
            .expect("reqwest client build failed");
        Self { owner: owner.to_string(), repo: repo.to_string(), token: token.to_string(), client }
    }

    fn auth(&self) -> String { format!("Bearer {}", self.token) }

    pub async fn test_token(&self) -> Result<bool> {
        let resp = self.client.get("https://api.github.com/user")
            .header("Authorization", self.auth())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send().await?;
        Ok(resp.status().is_success())
    }

    async fn list_issues(&self) -> Result<Vec<Issue>> {
        let mut all = Vec::new();
        let mut page: u32 = 1;
        loop {
            let url = format!(
                "https://api.github.com/repos/{}/{}/issues?state=all&per_page=100&page={}",
                self.owner, self.repo, page
            );
            let batch: Vec<Issue> = self.client.get(&url)
                .header("Authorization", self.auth())
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send().await?.error_for_status()?.json().await?;
            if batch.is_empty() { break; }
            page += 1;
            let had = batch.len();
            all.extend(batch.into_iter().filter(|i| i.pull_request.is_none()));
            if had < 100 { break; } // last page
            if all.len() >= 2000 { break; } // safety cap
        }
        Ok(all)
    }

    pub async fn create_issue(&self, title: &str, body: &str, labels: &[String]) -> Result<u64> {
        let url = format!("https://api.github.com/repos/{}/{}/issues", self.owner, self.repo);
        let resp: Issue = self.client.post(&url)
            .header("Authorization", self.auth())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&serde_json::json!({ "title": title, "body": body, "labels": labels }))
            .send().await?.error_for_status()?.json().await?;
        Ok(resp.number)
    }

    pub async fn update_issue(
        &self, number: u64,
        title: Option<&str>, body: Option<&str>,
        state: Option<&str>, labels: Option<&[String]>,
    ) -> Result<()> {
        let url = format!("https://api.github.com/repos/{}/{}/issues/{}", self.owner, self.repo, number);
        let mut m = serde_json::Map::new();
        if let Some(t) = title  { m.insert("title".into(), t.into()); }
        if let Some(b) = body   { m.insert("body".into(), b.into()); }
        if let Some(s) = state  { m.insert("state".into(), s.into()); }
        if let Some(l) = labels {
            m.insert("labels".into(), serde_json::Value::Array(
                l.iter().map(|s| serde_json::Value::String(s.clone())).collect()
            ));
        }
        self.client.patch(&url)
            .header("Authorization", self.auth())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&serde_json::Value::Object(m))
            .send().await?.error_for_status()?;
        Ok(())
    }

    async fn list_issue_comments(&self, issue_number: u64) -> Result<Vec<GhComment>> {
        let mut all = Vec::new();
        let mut page: u32 = 1;
        loop {
            let url = format!(
                "https://api.github.com/repos/{}/{}/issues/{}/comments?per_page=100&page={}",
                self.owner, self.repo, issue_number, page
            );
            let batch: Vec<GhComment> = self.client.get(&url)
                .header("Authorization", self.auth())
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send().await?.error_for_status()?.json().await?;
            let had = batch.len();
            all.extend(batch);
            if had < 100 { break; }
            page += 1;
        }
        Ok(all)
    }

    async fn create_issue_comment(&self, issue_number: u64, body: &str) -> Result<u64> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/comments",
            self.owner, self.repo, issue_number
        );
        let resp: GhComment = self.client.post(&url)
            .header("Authorization", self.auth())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&serde_json::json!({ "body": body }))
            .send().await?.error_for_status()?.json().await?;
        Ok(resp.id)
    }

    async fn download_bytes(&self, url: &str) -> Result<Vec<u8>> {
        // GitHub asset URLs redirect to signed S3 URLs.
        // We need the auth header for the first request (GitHub) but NOT for the
        // second (S3): sending Authorization to S3 on a signed URL causes 400.
        // So we disable automatic redirects, follow manually, stripping auth.
        let no_redirect = reqwest::Client::builder()
            .user_agent("monotask-github/0.4")
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        let resp = no_redirect.get(url)
            .header("Authorization", self.auth())
            .header("Accept", "application/vnd.github+json")
            .send().await?;

        let final_resp = if resp.status().is_redirection() {
            let location = resp.headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow::anyhow!("redirect with no Location header for {url}"))?
                .to_string();
            // Follow redirect without auth — S3 signed URLs must not have extra auth
            self.client.get(&location).send().await?.error_for_status()?
        } else {
            resp.error_for_status()?
        };

        Ok(final_resp.bytes().await?.to_vec())
    }

    /// Upload binary (base64-encoded) to GitHub Contents API.
    /// Returns the `download_url` from the response.
    async fn upload_file(&self, path: &str, data_b64: &str, message: &str) -> Result<String> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            self.owner, self.repo, path
        );
        #[derive(serde::Deserialize)]
        struct UploadResp { content: UploadContent }
        #[derive(serde::Deserialize)]
        struct UploadContent { download_url: Option<String> }
        let resp: UploadResp = self.client.put(&url)
            .header("Authorization", self.auth())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&serde_json::json!({ "message": message, "content": data_b64 }))
            .send().await?.error_for_status()?.json().await?;
        resp.content.download_url
            .ok_or_else(|| anyhow::anyhow!("no download_url in upload response for {path}"))
    }

    pub async fn ensure_label_exists(&self, name: &str, color: &str) -> Result<()> {
        let url = format!("https://api.github.com/repos/{}/{}/labels", self.owner, self.repo);
        let resp = self.client.post(&url)
            .header("Authorization", self.auth())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&serde_json::json!({ "name": name, "color": color }))
            .send().await?;
        // 201 created or 422 already exists — both are fine
        let status = resp.status();
        if !status.is_success() && status.as_u16() != 422 {
            resp.error_for_status()?;
        }
        Ok(())
    }
}

// ── Image sync helpers ────────────────────────────────────────────────────────

/// Extract `(alt, url)` pairs from both markdown `![alt](url)` and HTML `<img src="url">` tags.
fn extract_image_urls(text: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();

    // ── Markdown: ![alt](url) ─────────────────────────────────────────────
    let mut remaining = text;
    while let Some(pos) = remaining.find("![") {
        remaining = &remaining[pos + 2..];
        let alt_end = match remaining.find("](") {
            Some(p) => p,
            None => continue,
        };
        let alt = remaining[..alt_end].to_string();
        remaining = &remaining[alt_end + 2..];
        let url_end = match remaining.find(')') {
            Some(p) => p,
            None => continue,
        };
        let url = &remaining[..url_end];
        remaining = &remaining[url_end + 1..];
        if url.starts_with("http://") || url.starts_with("https://") {
            results.push((alt, url.to_string()));
        }
    }

    // ── HTML: <img ... src="url" ... alt="alt" ... /> ────────────────────
    let mut rem = text;
    while let Some(tag_start) = rem.find("<img ") {
        rem = &rem[tag_start + 5..];
        let tag_end = match rem.find('>') {
            Some(p) => p,
            None => continue,
        };
        let tag_body = &rem[..tag_end];
        rem = &rem[tag_end + 1..];

        let src = extract_attr(tag_body, "src");
        let alt = extract_attr(tag_body, "alt").unwrap_or_default();
        if let Some(url) = src {
            if url.starts_with("http://") || url.starts_with("https://") {
                results.push((alt, url));
            }
        }
    }

    results
}

/// Extract the value of a quoted HTML attribute from a tag body string.
fn extract_attr(tag_body: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = tag_body.find(needle.as_str())? + needle.len();
    let end = tag_body[start..].find('"')? + start;
    Some(tag_body[start..end].to_string())
}

/// Remove all `![alt](url)` and `<img ...>` image patterns, trimming trailing whitespace.
fn strip_image_markdown(text: &str) -> String {
    // Strip markdown images
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(pos) = remaining.find("![") {
        let rest = &remaining[pos + 2..];
        let alt_end = match rest.find("](") {
            Some(p) => p,
            None => { result.push_str(&remaining[..pos + 2]); remaining = rest; continue; }
        };
        let url_start = alt_end + 2;
        let url_end = match rest[url_start..].find(')') {
            Some(p) => url_start + p,
            None => { result.push_str(&remaining[..pos + 2]); remaining = rest; continue; }
        };
        result.push_str(&remaining[..pos]);
        remaining = &rest[url_end + 1..];
    }
    result.push_str(remaining);

    // Strip HTML <img ...> tags
    let mut result2 = String::with_capacity(result.len());
    let mut rem = result.as_str();
    while let Some(pos) = rem.find("<img ") {
        result2.push_str(&rem[..pos]);
        rem = &rem[pos + 5..];
        if let Some(end) = rem.find('>') {
            rem = &rem[end + 1..];
        }
    }
    result2.push_str(rem);
    result2.trim().to_string()
}

/// Guess MIME type from a file extension.
fn mime_from_ext(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif"          => "image/gif",
        "webp"         => "image/webp",
        "svg"          => "image/svg+xml",
        "bmp"          => "image/bmp",
        _              => "image/png",
    }
}

/// Compose the GitHub issue body from description + appended image markdown.
fn build_github_body(description: &str, image_attachments: &[(String, String)]) -> String {
    if image_attachments.is_empty() {
        return description.to_string();
    }
    let mut body = description.to_string();
    if !body.is_empty() { body.push_str("\n\n"); }
    for (name, url) in image_attachments {
        body.push_str(&format!("![{name}]({url})\n"));
    }
    body.trim_end().to_string()
}

/// Returns `(id, name, mime, Option<github_image_url>)` for every attachment on a card.
fn list_attachments_raw(doc: &AutoCommit, card_id: &str) -> Vec<(String, String, String, Option<String>)> {
    let mut result = Vec::new();
    let card_obj = match monotask_core::card::get_card_obj(doc, card_id) {
        Ok(o) => o, Err(_) => return result,
    };
    let attachments_map = match doc.get(&card_obj, "attachments") {
        Ok(Some((_, m))) => m, _ => return result,
    };
    for key in doc.keys(&attachments_map).map(|k| k.to_string()).collect::<Vec<_>>() {
        if let Ok(Some((_, entry))) = doc.get(&attachments_map, key.as_str()) {
            let name = monotask_core::get_string(doc, &entry, "name").ok().flatten().unwrap_or_default();
            let mime = monotask_core::get_string(doc, &entry, "mime").ok().flatten().unwrap_or_default();
            let gh_url = monotask_core::get_string(doc, &entry, "github_image_url").ok().flatten()
                .and_then(|s| if s.is_empty() { None } else { Some(s) });
            result.push((key, name, mime, gh_url));
        }
    }
    result
}

/// Write `github_image_url` onto an attachment CRDT entry.
fn set_attachment_github_url(doc: &mut AutoCommit, card_id: &str, attachment_id: &str, url: &str) {
    let card_obj = match monotask_core::card::get_card_obj(doc, card_id) {
        Ok(o) => o, Err(_) => return,
    };
    let attachments_map = match doc.get(&card_obj, "attachments") {
        Ok(Some((_, m))) => m, _ => return,
    };
    if let Ok(Some((_, entry))) = doc.get(&attachments_map, attachment_id) {
        let _ = doc.put(&entry, "github_image_url", url);
    }
}

// ── Sync ──────────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct SyncResult {
    pub pulled: usize,
    pub pushed: usize,
    pub closed: usize,
    pub errors: Vec<String>,
}

pub async fn sync_board(
    doc: &mut AutoCommit,
    token: &str,
    config: &GitHubConfig,
    actor_pk: &[u8],
) -> Result<SyncResult> {
    let client = GitHubClient::new(&config.owner, &config.repo, token);
    let mut result = SyncResult { pulled: 0, pushed: 0, closed: 0, errors: vec![] };

    // Capture sync start time BEFORE fetching issues so comments added during sync aren't missed next cycle
    let sync_start_ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let issues = client.list_issues().await.context("fetching GitHub issues")?;
    let issue_map: HashMap<u64, &Issue> = issues.iter().map(|i| (i.number, i)).collect();

    // ── Collect all read-only data before any mutations ───────────────────────

    // Build all_members from actor_card_seq so card number prefixes are correct on multi-member boards
    let all_members: Vec<Vec<u8>> = {
        let mut members: Vec<Vec<u8>> = match doc.get(ROOT, "actor_card_seq").ok().flatten() {
            Some((_, seq_map)) => doc.keys(&seq_map)
                .filter_map(|k| hex::decode(k).ok())
                .collect(),
            None => vec![],
        };
        if !members.contains(&actor_pk.to_vec()) {
            members.push(actor_pk.to_vec());
        }
        members
    };

    let columns = monotask_core::column::list_columns(doc).context("listing columns")?;
    let col_title_to_id: HashMap<String, String> = columns.iter()
        .map(|c| (c.title.clone(), c.id.clone())).collect();
    let col_id_to_title: HashMap<String, String> = columns.iter()
        .map(|c| (c.id.clone(), c.title.clone())).collect();

    // card_id → column_id (non-tombstoned cards per column card list)
    let mut card_to_col: HashMap<String, String> = HashMap::new();
    for col in &columns {
        if let Ok(Some(col_obj)) = monotask_core::column::find_column_obj(doc, &col.id) {
            if let Ok(card_ids_list) = monotask_core::column::get_card_ids_list(doc, &col_obj) {
                let len = doc.length(&card_ids_list);
                for i in 0..len {
                    if let Ok(Some((automerge::Value::Scalar(s), _))) = doc.get(&card_ids_list, i) {
                        if let ScalarValue::Str(t) = s.as_ref() {
                            card_to_col.insert(t.to_string(), col.id.clone());
                        }
                    }
                }
            }
        }
    }

    // All card IDs (including tombstoned) + issue number mapping
    let cards_map_obj = monotask_core::get_cards_map_readonly(doc).context("getting cards map")?;
    let all_card_ids: Vec<String> = doc.keys(&cards_map_obj).map(|k| k.to_string()).collect();

    let mut issue_num_to_card: HashMap<u64, String> = HashMap::new();
    for card_id in &all_card_ids {
        if let Some(num) = get_github_issue_number(doc, card_id) {
            issue_num_to_card.insert(num, card_id.clone());
        }
    }

    // ── Phase 1: Pull GitHub → CRDT ───────────────────────────────────────────
    for issue in &issues {
        if let Some(card_id) = issue_num_to_card.get(&issue.number).cloned() {
            let is_dead = monotask_core::card::is_tombstoned(doc, &card_id).unwrap_or(true);
            if is_dead { continue; }

            let synced_at = get_github_synced_at(doc, &card_id);
            let needs_pull = synced_at.as_deref()
                .map(|local_ts| issue.updated_at.as_str() > local_ts)
                .unwrap_or(true);

            if needs_pull {
                if let Err(e) = monotask_core::card::rename_card(doc, &card_id, &issue.title) {
                    result.errors.push(format!("rename_card {card_id}: {e}")); continue;
                }
                let body = issue.body.as_deref().unwrap_or("");
                // Store description with image markdown stripped (images become attachments in Phase 1c)
                let _ = monotask_core::card::set_description(doc, &card_id, &strip_image_markdown(body));

                // Move card if column changed
                let current_col = card_to_col.get(&card_id).cloned();
                let target_col = if issue.state == "closed" {
                    Some(config.done_column_id.clone())
                } else {
                    // Label-based column takes priority; fall back to first non-done column
                    // when card is currently sitting in the done column (issue was reopened).
                    issue.labels.iter().find_map(|l| col_title_to_id.get(&l.name).cloned())
                        .or_else(|| {
                            if current_col.as_deref() == Some(&config.done_column_id) {
                                columns.iter().find(|c| c.id != config.done_column_id).map(|c| c.id.clone())
                            } else {
                                None
                            }
                        })
                };
                if let (Some(from), Some(to)) = (current_col, target_col) {
                    if from != to {
                        if let Err(e) = monotask_core::column::move_card(doc, &card_id, &from, &to) {
                            result.errors.push(format!("move_card {card_id}: {e}"));
                        } else {
                            card_to_col.insert(card_id.clone(), to);
                        }
                    }
                }

                // Sync non-column labels
                sync_non_col_labels(doc, &card_id, &issue.labels, &col_title_to_id);

                let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
                let _ = set_github_synced_at(doc, &card_id, &now);
                result.pulled += 1;
            }
        } else {
            // No card linked to this issue yet.
            // Before creating a new card, try to match an existing unlinked local card by title
            // to prevent duplicates on first sync when local and GitHub cards mirror each other.
            let title_match = all_card_ids.iter().find(|cid| {
                if get_github_issue_number(doc, cid).is_some() { return false; }
                if monotask_core::card::is_tombstoned(doc, cid).unwrap_or(true) { return false; }
                monotask_core::card::read_card(doc, cid)
                    .map(|c| c.title == issue.title)
                    .unwrap_or(false)
            }).cloned();

            let (linked_card_id, is_new) = if let Some(existing_id) = title_match {
                (existing_id, false)
            } else {
                let target_col_id = if issue.state == "closed" {
                    config.done_column_id.clone()
                } else {
                    issue.labels.iter()
                        .find_map(|l| col_title_to_id.get(&l.name).cloned())
                        .or_else(|| columns.first().map(|c| c.id.clone()))
                        .unwrap_or_else(|| config.done_column_id.clone())
                };
                match monotask_core::card::create_card(doc, &target_col_id, &issue.title, actor_pk, &all_members) {
                    Ok(card) => { card_to_col.insert(card.id.clone(), target_col_id); (card.id, true) }
                    Err(e) => { result.errors.push(format!("create_card for issue #{}: {e}", issue.number)); continue; }
                }
            };

            let body = issue.body.as_deref().unwrap_or("");
            let stripped = strip_image_markdown(body);
            if !stripped.is_empty() {
                let _ = monotask_core::card::set_description(doc, &linked_card_id, &stripped);
            }
            let _ = set_github_issue_number(doc, &linked_card_id, issue.number);
            let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let _ = set_github_synced_at(doc, &linked_card_id, &now);
            sync_non_col_labels(doc, &linked_card_id, &issue.labels, &col_title_to_id);

            // If matched to existing card, move it to the correct column
            if !is_new {
                let target_col_id = if issue.state == "closed" {
                    config.done_column_id.clone()
                } else {
                    issue.labels.iter()
                        .find_map(|l| col_title_to_id.get(&l.name).cloned())
                        .or_else(|| card_to_col.get(&linked_card_id).cloned())
                        .unwrap_or_else(|| config.done_column_id.clone())
                };
                if let Some(from) = card_to_col.get(&linked_card_id).cloned() {
                    if from != target_col_id {
                        let _ = monotask_core::column::move_card(doc, &linked_card_id, &from, &target_col_id);
                        card_to_col.insert(linked_card_id.clone(), target_col_id);
                    }
                }
            }

            issue_num_to_card.insert(issue.number, linked_card_id);
            result.pulled += 1;
        }
    }

    // ── Phase 1b: Pull GitHub comments → CRDT ────────────────────────────────
    // Only fetch comments for issues whose updated_at is newer than our last sync
    // (or that have never been synced). This avoids O(n) API calls every poll cycle.
    let last_sync_ts = config.last_sync.as_deref().unwrap_or("");
    for issue in &issues {
        let card_id = match issue_num_to_card.get(&issue.number) {
            Some(id) => id.clone(),
            None => continue,
        };
        if monotask_core::card::is_tombstoned(doc, &card_id).unwrap_or(true) { continue; }

        let already_imported = card_github_comment_ids(doc, &card_id);

        // Determine whether to fetch comments for this issue:
        // - Skip if GitHub reports zero comments AND we have none locally (nothing to import)
        // - Skip if issue hasn't changed since last sync (timestamp-based incremental)
        // - Always fetch if we have 0 imported but GitHub says >0 (recovery from failed prior imports)
        if already_imported.is_empty() && issue.comments == 0 {
            continue; // GitHub confirms no comments — no API call needed
        }
        if !already_imported.is_empty() && !last_sync_ts.is_empty() && issue.updated_at.as_str() <= last_sync_ts {
            continue; // nothing new since last sync
        }

        let gh_comments = match client.list_issue_comments(issue.number).await {
            Ok(c) => c,
            Err(e) => {
                result.errors.push(format!("list_comments #{}: {e}", issue.number));
                continue;
            }
        };

        for gh_comment in &gh_comments {
            if already_imported.contains_key(&gh_comment.id) { continue; }

            let login = gh_comment.user.as_ref().map(|u| u.login.as_str()).unwrap_or("ghost");
            let avatar_url = gh_comment.user.as_ref().map(|u| u.avatar_url.as_str()).unwrap_or("");
            let author_label = format!("@{}", login);
            let body_text = gh_comment.body.as_deref().unwrap_or("");
            let text = format!("{}: {}", author_label, body_text);
            match monotask_core::comment::add_comment(doc, &card_id, &text, &author_label) {
                Ok(local) => {
                    set_comment_github_id(doc, &card_id, &local.id, gh_comment.id);
                    if !avatar_url.is_empty() {
                        let _ = monotask_core::comment::set_comment_avatar_url(doc, &card_id, &local.id, avatar_url);
                    }
                    result.pulled += 1;
                }
                Err(e) => result.errors.push(format!(
                    "add_comment card={card_id} gh_comment={}: {e}", gh_comment.id
                )),
            }
        }
    }

    // ── Phase 1c: Pull images from GitHub issue bodies → CRDT attachments ────
    for issue in &issues {
        let card_id = match issue_num_to_card.get(&issue.number) {
            Some(id) => id.clone(),
            None => continue,
        };
        if monotask_core::card::is_tombstoned(doc, &card_id).unwrap_or(true) { continue; }
        let body = issue.body.as_deref().unwrap_or("");
        if body.is_empty() { continue; }
        let image_urls = extract_image_urls(body);
        if image_urls.is_empty() { continue; }

        // Collect already-imported GitHub image URLs so we skip them
        let existing_gh_urls: HashSet<String> = list_attachments_raw(doc, &card_id)
            .into_iter().filter_map(|(_, _, _, gh)| gh).collect();

        for (alt, url) in &image_urls {
            if existing_gh_urls.contains(url) { continue; }
            let bytes = match client.download_bytes(url).await {
                Ok(b) => b,
                Err(e) => { result.errors.push(format!("download image {url}: {e}")); continue; }
            };
            let data_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let ext = url.rsplit(['.', '/']).next().unwrap_or("png");
            let name = {
                let s: String = alt.chars()
                    .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
                    .collect();
                if s.is_empty() { format!("{}.{ext}", uuid::Uuid::new_v4()) } else { s }
            };
            let mime = mime_from_ext(ext);
            let att_id = uuid::Uuid::new_v4().to_string();
            match monotask_core::card::attach_image(doc, &card_id, &att_id, &name, mime, &data_b64) {
                Ok(()) => {
                    set_attachment_github_url(doc, &card_id, &att_id, url);
                    result.pulled += 1;
                }
                Err(e) => result.errors.push(format!("attach_image {card_id}: {e}")),
            }
        }
    }

    // ── Phase 2c: Upload new local image attachments → GitHub ─────────────────
    // Collect all upload tasks (read-only pass), then execute (write pass).
    // We must do this BEFORE Phase 2 so Phase 2 can embed the raw URLs in the issue body.
    type UploadTask = (String, u64, String, String, String); // (card_id, issue_num, att_id, name, data_b64)
    let mut upload_tasks: Vec<UploadTask> = Vec::new();
    for card_id in &all_card_ids {
        if monotask_core::card::is_tombstoned(doc, card_id).unwrap_or(true) { continue; }
        let issue_num = match get_github_issue_number(doc, card_id) {
            Some(n) => n, None => continue,
        };
        let atts = list_attachments_raw(doc, card_id);
        let new_image_atts: Vec<_> = atts.into_iter()
            .filter(|(_, _, mime, gh)| gh.is_none() && mime.starts_with("image/"))
            .collect();
        if new_image_atts.is_empty() { continue; }
        let card = match monotask_core::card::read_card(doc, card_id) {
            Ok(c) => c, Err(_) => continue,
        };
        for (att_id, name, _, _) in new_image_atts {
            let data_b64 = card.attachments.get(&att_id)
                .map(|a| a.data_b64.clone()).unwrap_or_default();
            if data_b64.is_empty() { continue; }
            upload_tasks.push((card_id.clone(), issue_num, att_id, name, data_b64));
        }
    }
    for (card_id, _, att_id, name, data_b64) in upload_tasks {
        let filename = format!("{att_id}-{name}");
        let path = format!(".monotask-attachments/{filename}");
        match client.upload_file(&path, &data_b64, &format!("Add image {name} from monotask")).await {
            Ok(raw_url) => {
                set_attachment_github_url(doc, &card_id, &att_id, &raw_url);
                result.pushed += 1;
            }
            Err(e) => result.errors.push(format!("upload image {name}: {e}")),
        }
    }

    // ── Phase 2: Push CRDT → GitHub ───────────────────────────────────────────
    let label_colors = ["0075ca","e4e669","d93f0b","0e8a16","1d76db","5319e7","b60205","f9d0c4"];
    for (i, col) in columns.iter().enumerate() {
        if col.id != config.done_column_id {
            let color = label_colors[i % label_colors.len()];
            if let Err(e) = client.ensure_label_exists(&col.title, color).await {
                result.errors.push(format!("ensure_label '{}': {e}", col.title));
            }
        }
    }

    for card_id in &all_card_ids {
        let is_dead = monotask_core::card::is_tombstoned(doc, card_id).unwrap_or(true);
        if is_dead { continue; }

        let card = match monotask_core::card::read_card(doc, card_id) {
            Ok(c) => c,
            Err(e) => { result.errors.push(format!("read_card {card_id}: {e}")); continue; }
        };

        let col_id = card_to_col.get(card_id).cloned().unwrap_or_default();
        let is_done = col_id == config.done_column_id;
        let state_str = if is_done { "closed" } else { "open" };

        // Build labels: column label (if not done) + card's own labels
        let mut issue_labels: Vec<String> = card.labels.clone();
        if !is_done {
            if let Some(col_title) = col_id_to_title.get(&col_id) {
                if !issue_labels.contains(col_title) {
                    issue_labels.insert(0, col_title.clone());
                }
            }
        }

        if let Some(issue_num) = get_github_issue_number(doc, card_id) {
            if let Some(&issue) = issue_map.get(&issue_num) {
                // Build expected body: description + image markdown for all pushed attachments
                let image_atts: Vec<(String, String)> = list_attachments_raw(doc, card_id)
                    .into_iter()
                    .filter_map(|(_, name, _, gh)| gh.map(|url| (name, url)))
                    .collect();
                let expected_body = build_github_body(&card.description, &image_atts);
                let gh_label_set: HashSet<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
                let our_label_set: HashSet<&str> = issue_labels.iter().map(|s| s.as_str()).collect();
                let needs_push = issue.title != card.title
                    || issue.body.as_deref().unwrap_or("") != expected_body.as_str()
                    || issue.state != state_str
                    || gh_label_set != our_label_set;
                if needs_push {
                    if let Err(e) = client.update_issue(
                        issue_num, Some(&card.title), Some(&expected_body),
                        Some(state_str), Some(&issue_labels),
                    ).await {
                        result.errors.push(format!("update_issue #{issue_num}: {e}"));
                    } else {
                        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
                        let _ = set_github_synced_at(doc, card_id, &now);
                        result.pushed += 1;
                    }
                }
            }
        } else {
            let image_atts: Vec<(String, String)> = list_attachments_raw(doc, card_id)
                .into_iter()
                .filter_map(|(_, name, _, gh)| gh.map(|url| (name, url)))
                .collect();
            let full_body = build_github_body(&card.description, &image_atts);
            match client.create_issue(&card.title, &full_body, &issue_labels).await {
                Ok(num) => {
                    if let Err(e) = set_github_issue_number(doc, card_id, num) {
                        result.errors.push(format!("set_github_issue_number {card_id}: {e}"));
                    } else {
                        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
                        let _ = set_github_synced_at(doc, card_id, &now);
                        if is_done {
                            let _ = client.update_issue(num, None, None, Some("closed"), None).await;
                        }
                        result.pushed += 1;
                    }
                }
                Err(e) => result.errors.push(format!("create_issue for {card_id}: {e}")),
            }
        }
    }

    // ── Phase 2b: Push new local comments → GitHub ───────────────────────────
    // For every card with a linked issue, find comments without a github_comment_id
    // and post them as new GitHub comments.
    for card_id in &all_card_ids {
        if monotask_core::card::is_tombstoned(doc, card_id).unwrap_or(true) { continue; }
        let issue_num = match get_github_issue_number(doc, card_id) {
            Some(n) => n,
            None => continue,
        };

        let local_comments = match monotask_core::comment::list_comments(doc, card_id) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let already_pushed = card_local_comment_gh_ids(doc, card_id);

        for comment in &local_comments {
            if already_pushed.contains_key(&comment.id) { continue; }
            // Skip comments that were pulled from GitHub (they start with "@<login>: ")
            // by checking if they have a github_comment_id (they'd be in already_pushed above)
            // New local comments: push them to GitHub
            match client.create_issue_comment(issue_num, &comment.text).await {
                Ok(gh_id) => {
                    set_comment_github_id(doc, card_id, &comment.id, gh_id);
                    result.pushed += 1;
                }
                Err(e) => result.errors.push(format!(
                    "create_comment issue=#{issue_num} comment={}: {e}", comment.id
                )),
            }
        }
    }

    // ── Phase 3: Close deleted cards' issues ──────────────────────────────────
    for card_id in &all_card_ids {
        let is_dead = monotask_core::card::is_tombstoned(doc, card_id).unwrap_or(false);
        if !is_dead { continue; }
        if let Some(issue_num) = get_github_issue_number(doc, card_id) {
            if let Some(&issue) = issue_map.get(&issue_num) {
                if issue.state == "open" {
                    if let Err(e) = client.update_issue(issue_num, None, None, Some("closed"), None).await {
                        result.errors.push(format!("close_issue #{issue_num}: {e}"));
                    } else {
                        result.closed += 1;
                    }
                }
            }
        }
    }

    // Update last_sync to the timestamp captured before fetching, so issues updated during sync are re-checked next cycle
    let updated = GitHubConfig { last_sync: Some(sync_start_ts), ..config.clone() };
    let _ = set_github_config(doc, Some(&updated));

    Ok(result)
}

// ── Single-card targeted sync ─────────────────────────────────────────────────

pub async fn sync_single_card(
    doc: &mut AutoCommit,
    token: &str,
    config: &GitHubConfig,
    card_id: &str,
    actor_pk: &[u8],
) -> Result<SyncResult> {
    let mut result = SyncResult { pulled: 0, pushed: 0, closed: 0, errors: vec![] };

    let issue_number = match get_github_issue_number(doc, card_id) {
        Some(n) => n,
        None => return Err(anyhow::anyhow!("card has no linked GitHub issue")),
    };
    if monotask_core::card::is_tombstoned(doc, card_id).unwrap_or(true) {
        return Err(anyhow::anyhow!("card is archived"));
    }

    let client = GitHubClient::new(&config.owner, &config.repo, token);

    // ── Fetch the single issue ────────────────────────────────────────────────
    let url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}",
        config.owner, config.repo, issue_number
    );
    let issue: Issue = client.client.get(&url)
        .header("Authorization", client.auth())
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send().await?.error_for_status()?.json().await?;

    // ── Pull: update card from GitHub ─────────────────────────────────────────
    let synced_at = get_github_synced_at(doc, card_id);
    let needs_pull = synced_at.as_deref()
        .map(|local_ts| issue.updated_at.as_str() > local_ts)
        .unwrap_or(true);

    if needs_pull {
        let _ = monotask_core::card::rename_card(doc, card_id, &issue.title);
        let body = issue.body.as_deref().unwrap_or("");
        let _ = monotask_core::card::set_description(doc, card_id, &strip_image_markdown(body));

        // Columns
        let columns = monotask_core::column::list_columns(doc)?;
        let col_title_to_id: HashMap<String, String> = columns.iter()
            .map(|c| (c.title.clone(), c.id.clone())).collect();
        sync_non_col_labels(doc, card_id, &issue.labels, &col_title_to_id);
        result.pulled += 1;
    }

    // ── Pull: comments ────────────────────────────────────────────────────────
    let already_imported = card_github_comment_ids(doc, card_id);
    let gh_comments = client.list_issue_comments(issue_number).await
        .context("fetching issue comments")?;

    eprintln!("[sync_single_card] issue #{issue_number}: gh_comments={}, already_imported={}", gh_comments.len(), already_imported.len());

    for gh_comment in &gh_comments {
        eprintln!("[sync_single_card]   gh_comment id={} already={}", gh_comment.id, already_imported.contains_key(&gh_comment.id));
        if already_imported.contains_key(&gh_comment.id) { continue; }
        let login = gh_comment.user.as_ref().map(|u| u.login.as_str()).unwrap_or("ghost");
        let avatar_url = gh_comment.user.as_ref().map(|u| u.avatar_url.as_str()).unwrap_or("");
        let author_label = format!("@{}", login);
        let body_text = gh_comment.body.as_deref().unwrap_or("");
        let text = format!("{}: {}", author_label, body_text);
        eprintln!("[sync_single_card]   importing comment from @{login}: {:?}", &body_text[..body_text.len().min(60)]);
        match monotask_core::comment::add_comment(doc, card_id, &text, &author_label) {
            Ok(local) => {
                set_comment_github_id(doc, card_id, &local.id, gh_comment.id);
                if !avatar_url.is_empty() {
                    let _ = monotask_core::comment::set_comment_avatar_url(doc, card_id, &local.id, avatar_url);
                }
                result.pulled += 1;
            }
            Err(e) => result.errors.push(format!("add_comment: {e}")),
        }
    }
    eprintln!("[sync_single_card] done: pulled={} pushed={} errors={:?}", result.pulled, result.pushed, result.errors);

    // ── Push: update GitHub issue from CRDT ───────────────────────────────────
    let all_members: Vec<Vec<u8>> = {
        let mut m: Vec<Vec<u8>> = match doc.get(ROOT, "actor_card_seq").ok().flatten() {
            Some((_, seq_map)) => doc.keys(&seq_map)
                .filter_map(|k| hex::decode(k).ok()).collect(),
            None => vec![],
        };
        if !m.contains(&actor_pk.to_vec()) { m.push(actor_pk.to_vec()); }
        m
    };
    let _ = all_members; // used by create_card in full sync; not needed here

    let card = monotask_core::card::read_card(doc, card_id)?;
    let columns = monotask_core::column::list_columns(doc)?;
    let col_title_to_id: HashMap<String, String> = columns.iter()
        .map(|c| (c.title.clone(), c.id.clone())).collect();
    let col_id_to_title: HashMap<String, String> = columns.iter()
        .map(|c| (c.id.clone(), c.title.clone())).collect();

    // Determine current column
    let mut card_col_id = String::new();
    for col in &columns {
        if let Ok(Some(col_obj)) = monotask_core::column::find_column_obj(doc, &col.id) {
            if let Ok(card_ids_list) = monotask_core::column::get_card_ids_list(doc, &col_obj) {
                for i in 0..doc.length(&card_ids_list) {
                    if let Ok(Some((automerge::Value::Scalar(s), _))) = doc.get(&card_ids_list, i) {
                        if let ScalarValue::Str(t) = s.as_ref() {
                            if t.as_str() == card_id { card_col_id = col.id.clone(); break; }
                        }
                    }
                }
            }
        }
        if !card_col_id.is_empty() { break; }
    }

    let is_done = card_col_id == config.done_column_id;
    let state_str = if is_done { "closed" } else { "open" };
    let mut issue_labels: Vec<String> = card.labels.clone();
    if !is_done {
        if let Some(col_title) = col_id_to_title.get(&card_col_id) {
            if !issue_labels.contains(col_title) { issue_labels.insert(0, col_title.clone()); }
        }
    }

    // Ensure column label exists on GitHub
    if !is_done && !card_col_id.is_empty() {
        if let Some(col_title) = col_id_to_title.get(&card_col_id) {
            let _ = client.ensure_label_exists(col_title, "0075ca").await;
        }
    }

    client.update_issue(
        issue_number,
        Some(&card.title),
        Some(&card.description),
        Some(state_str),
        Some(&issue_labels),
    ).await.context("pushing to GitHub")?;

    // Push new local comments to GitHub
    let all_comments = monotask_core::comment::list_comments(doc, card_id)?;
    for comment in &all_comments {
        if already_imported.values().any(|local_id| local_id == &comment.id) { continue; }
        match client.create_issue_comment(issue_number, &comment.text).await {
            Ok(gh_id) => {
                set_comment_github_id(doc, card_id, &comment.id, gh_id);
                result.pushed += 1;
            }
            Err(e) => result.errors.push(format!("push_comment: {e}")),
        }
    }

    // Update synced_at timestamp
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let _ = set_github_synced_at(doc, card_id, &now);
    if result.pulled > 0 || result.pushed > 0 { result.pulled = result.pulled.max(1); }

    Ok(result)
}

fn sync_non_col_labels(
    doc: &mut AutoCommit,
    card_id: &str,
    issue_labels: &[GhLabel],
    col_title_to_id: &HashMap<String, String>,
) {
    let non_col: HashSet<String> = issue_labels.iter()
        .filter(|l| !col_title_to_id.contains_key(&l.name))
        .map(|l| l.name.clone())
        .collect();
    let current = monotask_core::card::read_card(doc, card_id)
        .map(|c| c.labels)
        .unwrap_or_default();
    // Remove labels that are gone (but not column labels — those are managed separately)
    for lbl in &current {
        if !col_title_to_id.contains_key(lbl) && !non_col.contains(lbl) {
            let _ = monotask_core::card::remove_label(doc, card_id, lbl);
        }
    }
    for lbl in &non_col {
        let _ = monotask_core::card::add_label(doc, card_id, lbl);
    }
}
