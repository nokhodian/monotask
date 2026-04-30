use automerge::{AutoCommit, ReadDoc, ScalarValue, transaction::Transactable, ROOT};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

// ── Token management ──────────────────────────────────────────────────────────

pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join("linear_token")
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
    let client = LinearClient::new(token);
    client.test_token().await
}

// ── Board config in CRDT ──────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LinearConfig {
    pub team_id: String,
    pub project_id: String,
    pub project_name: String,
    pub done_column_id: String,
    pub done_state_id: String, // default "completed" state id for pushing done cards
    pub last_sync: Option<String>,
}

pub fn get_linear_config(doc: &AutoCommit) -> Option<LinearConfig> {
    let team_id = monotask_core::get_string(doc, &ROOT, "linear_team_id").ok()??;
    let project_id = monotask_core::get_string(doc, &ROOT, "linear_project_id").ok()??;
    if team_id.is_empty() || project_id.is_empty() {
        return None;
    }
    let project_name = monotask_core::get_string(doc, &ROOT, "linear_project_name")
        .ok().flatten().unwrap_or_default();
    let done_column_id = monotask_core::get_string(doc, &ROOT, "linear_done_column_id")
        .ok().flatten().unwrap_or_default();
    let done_state_id = monotask_core::get_string(doc, &ROOT, "linear_done_state_id")
        .ok().flatten().unwrap_or_default();
    let last_sync = monotask_core::get_string(doc, &ROOT, "linear_last_sync")
        .ok().flatten()
        .and_then(|s| if s.is_empty() { None } else { Some(s) });
    Some(LinearConfig { team_id, project_id, project_name, done_column_id, done_state_id, last_sync })
}

pub fn set_linear_config(doc: &mut AutoCommit, config: Option<&LinearConfig>) -> monotask_core::Result<()> {
    match config {
        Some(c) => {
            doc.put(ROOT, "linear_team_id", c.team_id.as_str())?;
            doc.put(ROOT, "linear_project_id", c.project_id.as_str())?;
            doc.put(ROOT, "linear_project_name", c.project_name.as_str())?;
            doc.put(ROOT, "linear_done_column_id", c.done_column_id.as_str())?;
            doc.put(ROOT, "linear_done_state_id", c.done_state_id.as_str())?;
            doc.put(ROOT, "linear_last_sync", c.last_sync.as_deref().unwrap_or(""))?;
        }
        None => {
            doc.put(ROOT, "linear_team_id", "")?;
            doc.put(ROOT, "linear_project_id", "")?;
            doc.put(ROOT, "linear_project_name", "")?;
            doc.put(ROOT, "linear_done_column_id", "")?;
            doc.put(ROOT, "linear_done_state_id", "")?;
            doc.put(ROOT, "linear_last_sync", "")?;
        }
    }
    Ok(())
}

// ── Card linear fields in CRDT ────────────────────────────────────────────────

pub fn get_linear_issue_id(doc: &AutoCommit, card_id: &str) -> Option<String> {
    let card_obj = monotask_core::card::get_card_obj(doc, card_id).ok()?;
    monotask_core::get_string(doc, &card_obj, "linear_issue_id")
        .ok().flatten()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
}

pub fn set_linear_issue_id(doc: &mut AutoCommit, card_id: &str, id: &str) -> monotask_core::Result<()> {
    let cards_map = monotask_core::get_cards_map(doc)?;
    let card_obj = match doc.get(&cards_map, card_id)? {
        Some((_, id_obj)) => id_obj,
        None => return Err(monotask_core::Error::NotFound(card_id.into())),
    };
    doc.put(&card_obj, "linear_issue_id", id)?;
    Ok(())
}

pub fn get_linear_issue_identifier(doc: &AutoCommit, card_id: &str) -> Option<String> {
    let card_obj = monotask_core::card::get_card_obj(doc, card_id).ok()?;
    monotask_core::get_string(doc, &card_obj, "linear_issue_identifier")
        .ok().flatten()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
}

pub fn set_linear_issue_identifier(doc: &mut AutoCommit, card_id: &str, identifier: &str) -> monotask_core::Result<()> {
    let cards_map = monotask_core::get_cards_map(doc)?;
    let card_obj = match doc.get(&cards_map, card_id)? {
        Some((_, id_obj)) => id_obj,
        None => return Err(monotask_core::Error::NotFound(card_id.into())),
    };
    doc.put(&card_obj, "linear_issue_identifier", identifier)?;
    Ok(())
}

pub fn get_linear_synced_at(doc: &AutoCommit, card_id: &str) -> Option<String> {
    let card_obj = monotask_core::card::get_card_obj(doc, card_id).ok()?;
    monotask_core::get_string(doc, &card_obj, "linear_synced_at")
        .ok().flatten()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
}

pub fn set_linear_synced_at(doc: &mut AutoCommit, card_id: &str, ts: &str) -> monotask_core::Result<()> {
    let cards_map = monotask_core::get_cards_map(doc)?;
    let card_obj = match doc.get(&cards_map, card_id)? {
        Some((_, id_obj)) => id_obj,
        None => return Err(monotask_core::Error::NotFound(card_id.into())),
    };
    doc.put(&card_obj, "linear_synced_at", ts)?;
    Ok(())
}

// ── Comment linear_id tracking in CRDT ───────────────────────────────────────

/// Returns map of linear_comment_id → local comment id for a card.
fn card_linear_comment_ids(doc: &AutoCommit, card_id: &str) -> HashMap<String, String> {
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
            let linear_id = match monotask_core::get_string(doc, &c_obj, "linear_comment_id") {
                Ok(Some(s)) if !s.is_empty() => s,
                _ => continue,
            };
            let local_id = match monotask_core::get_string(doc, &c_obj, "id") {
                Ok(Some(s)) => s,
                _ => continue,
            };
            map.insert(linear_id, local_id);
        }
    }
    map
}

/// Returns map of local comment id → linear_comment_id for comments that have been pushed.
fn card_local_comment_linear_ids(doc: &AutoCommit, card_id: &str) -> HashMap<String, String> {
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
            let linear_id = match monotask_core::get_string(doc, &c_obj, "linear_comment_id") {
                Ok(Some(s)) if !s.is_empty() => s,
                _ => continue,
            };
            let local_id = match monotask_core::get_string(doc, &c_obj, "id") {
                Ok(Some(s)) => s,
                _ => continue,
            };
            map.insert(local_id, linear_id);
        }
    }
    map
}

fn set_comment_linear_id(doc: &mut AutoCommit, card_id: &str, local_comment_id: &str, linear_comment_id: &str) {
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
            let id = match monotask_core::get_string(doc, &c_obj, "id") {
                Ok(Some(s)) => s,
                _ => continue,
            };
            if id == local_comment_id {
                let _ = doc.put(&c_obj, "linear_comment_id", linear_comment_id);
                return;
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_done_state(state_type: &str, state_name: &str) -> bool {
    state_type == "completed"
        || state_type == "cancelled"
        || state_name.to_lowercase() == "iced"
}

/// Build description body with optional assignee footer.
fn build_description_with_assignee(desc: &str, assignee_name: Option<&str>) -> String {
    let base = strip_assignee_footer(desc);
    match assignee_name {
        Some(name) if !name.is_empty() => format!("{}\n\n---\n_Assigned to: {}_", base, name),
        _ => base,
    }
}

/// Strip the "Assigned to:" footer we append when pulling from Linear.
fn strip_assignee_footer(desc: &str) -> String {
    if let Some(idx) = desc.rfind("\n\n---\n_Assigned to:") {
        desc[..idx].trim_end().to_string()
    } else {
        desc.trim_end().to_string()
    }
}

/// Map Linear priority (0-4) to impact/effort such that compute_priority(impact, effort) == priority.
/// Formula: compute_priority = (impact + 10 - effort) / 2 (integer division)
/// Solution: impact = priority, effort = 10 - priority → cp = (priority + 10 - (10-priority)) / 2 = priority ✓
fn linear_priority_to_impact_effort(priority: u8) -> Option<(u8, u8)> {
    if priority == 0 { return None; }
    let p = priority.min(4);
    Some((p, 10 - p))
}

/// Map compute_priority result to Linear priority (clamp to 0-4, 0 = no priority).
fn compute_priority_to_linear(impact: u8, effort: u8) -> u8 {
    let cp = monotask_core::card::compute_priority(impact, effort);
    cp.min(4)
}

// ── API types ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct LinearTeam {
    pub id: String,
    pub name: String,
    pub key: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct LinearProject {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct LinearWorkflowState {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub position: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LinearUser {
    id: String,
    name: String,
    #[allow(dead_code)]
    email: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LinearLabel {
    id: String,
    name: String,
    color: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LinearComment {
    id: String,
    body: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    user: Option<LinearUser>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LinearIssue {
    id: String,
    identifier: String,
    title: String,
    description: Option<String>,
    priority: u8,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    state: LinearStateRef,
    assignee: Option<LinearUser>,
    labels: LinearLabelNodes,
    comments: LinearCommentNodes,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LinearStateRef {
    id: String,
    name: String,
    #[serde(rename = "type")]
    type_: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LinearLabelNodes {
    nodes: Vec<LinearLabel>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LinearCommentNodes {
    nodes: Vec<LinearComment>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SyncResult {
    pub pulled: usize,
    pub pushed: usize,
    pub closed: usize,
    pub errors: Vec<String>,
}

// ── HTTP client ───────────────────────────────────────────────────────────────

pub struct LinearClient {
    token: String,
    client: reqwest::Client,
}

impl LinearClient {
    pub fn new(token: &str) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("monotask/1.0")
            .build()
            .expect("reqwest client");
        Self { token: token.to_string(), client }
    }

    fn auth(&self) -> String {
        self.token.clone()
    }

    async fn graphql<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T> {
        let body = serde_json::json!({ "query": query, "variables": variables });
        let resp = self.client
            .post("https://api.linear.app/graphql")
            .header("Authorization", self.auth())
            .header("Content-Type", "application/json")
            .json(&body)
            .send().await?
            .error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        if let Some(errors) = json.get("errors") {
            anyhow::bail!("Linear API error: {}", errors);
        }
        let data = json.get("data")
            .ok_or_else(|| anyhow::anyhow!("No data in Linear response"))?;
        Ok(serde_json::from_value(data.clone())?)
    }

    pub async fn test_token(&self) -> Result<bool> {
        let q = "query { viewer { id } }";
        let result = self.graphql::<serde_json::Value>(q, serde_json::json!({})).await;
        Ok(result.is_ok())
    }

    pub async fn list_teams(&self) -> Result<Vec<LinearTeam>> {
        let q = "query { teams { nodes { id name key } } }";
        let data: serde_json::Value = self.graphql(q, serde_json::json!({})).await?;
        let nodes = data["teams"]["nodes"].clone();
        Ok(serde_json::from_value(nodes)?)
    }

    pub async fn list_projects(&self, team_id: &str) -> Result<Vec<LinearProject>> {
        let q = r#"
            query($teamId: ID!) {
              projects(filter: { accessibleTeams: { id: { eq: $teamId } } }, first: 100) {
                nodes { id name }
              }
            }
        "#;
        let data: serde_json::Value = self.graphql(q, serde_json::json!({"teamId": team_id})).await?;
        let nodes = data["projects"]["nodes"].clone();
        Ok(serde_json::from_value(nodes)?)
    }

    pub async fn list_workflow_states(&self, team_id: &str) -> Result<Vec<LinearWorkflowState>> {
        let q = r#"
            query($teamId: ID!) {
              workflowStates(
                filter: { team: { id: { eq: $teamId } } }
                orderBy: position
                first: 100
              ) {
                nodes { id name type position }
              }
            }
        "#;
        let data: serde_json::Value = self.graphql(q, serde_json::json!({"teamId": team_id})).await?;
        let nodes = data["workflowStates"]["nodes"].clone();
        Ok(serde_json::from_value(nodes)?)
    }

    async fn list_issues(&self, project_id: &str, since: Option<&str>) -> Result<Vec<LinearIssue>> {
        let q = r#"
            query($projectId: ID!, $filter: IssueFilter, $after: String) {
              issues(filter: $filter, first: 250, after: $after) {
                nodes {
                  id identifier title description priority updatedAt
                  state { id name type }
                  assignee { id name email }
                  labels { nodes { id name color } }
                  comments(first: 100) { nodes { id body updatedAt user { id name email } } }
                }
                pageInfo { hasNextPage endCursor }
              }
            }
        "#;
        let mut filter = serde_json::json!({
            "project": { "id": { "eq": project_id } }
        });
        if let Some(ts) = since {
            filter["updatedAt"] = serde_json::json!({ "gt": ts });
        }

        let mut all_issues = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let vars = serde_json::json!({
                "projectId": project_id,
                "filter": filter,
                "after": cursor
            });
            let data: serde_json::Value = self.graphql(q, vars).await?;
            let page = &data["issues"];
            let nodes: Vec<LinearIssue> = serde_json::from_value(page["nodes"].clone())?;
            all_issues.extend(nodes);
            let has_next = page["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
            if !has_next { break; }
            cursor = page["pageInfo"]["endCursor"].as_str().map(|s| s.to_string());
        }
        Ok(all_issues)
    }

    async fn create_issue(&self, input: &serde_json::Value) -> Result<(String, String)> {
        let q = r#"
            mutation($input: IssueCreateInput!) {
              issueCreate(input: $input) {
                success issue { id identifier }
              }
            }
        "#;
        let data: serde_json::Value = self.graphql(q, serde_json::json!({"input": input})).await?;
        let issue = &data["issueCreate"]["issue"];
        Ok((
            issue["id"].as_str().unwrap_or("").to_string(),
            issue["identifier"].as_str().unwrap_or("").to_string(),
        ))
    }

    async fn update_issue(&self, issue_id: &str, input: &serde_json::Value) -> Result<()> {
        let q = r#"
            mutation($id: String!, $input: IssueUpdateInput!) {
              issueUpdate(id: $id, input: $input) { success }
            }
        "#;
        self.graphql::<serde_json::Value>(q, serde_json::json!({"id": issue_id, "input": input})).await?;
        Ok(())
    }

    async fn create_comment(&self, issue_id: &str, body: &str) -> Result<String> {
        let q = r#"
            mutation($input: CommentCreateInput!) {
              commentCreate(input: $input) { success comment { id } }
            }
        "#;
        let data: serde_json::Value = self.graphql(q, serde_json::json!({
            "input": { "issueId": issue_id, "body": body }
        })).await?;
        Ok(data["commentCreate"]["comment"]["id"].as_str().unwrap_or("").to_string())
    }

    async fn get_issue(&self, issue_id: &str) -> Result<LinearIssue> {
        let q = r#"
            query($id: String!) {
              issue(id: $id) {
                id identifier title description priority updatedAt
                state { id name type }
                assignee { id name email }
                labels { nodes { id name color } }
                comments(first: 100) { nodes { id body updatedAt user { id name email } } }
              }
            }
        "#;
        let data: serde_json::Value = self.graphql(q, serde_json::json!({"id": issue_id})).await?;
        Ok(serde_json::from_value(data["issue"].clone())?)
    }

    async fn ensure_label_exists(&self, team_id: &str, name: &str, color: &str) -> Result<String> {
        // First: search existing labels
        let q = r#"
            query($teamId: ID!, $name: String!) {
              issueLabels(filter: { team: { id: { eq: $teamId } }, name: { eq: $name } }) {
                nodes { id name }
              }
            }
        "#;
        let data: serde_json::Value = self.graphql(q, serde_json::json!({
            "teamId": team_id, "name": name
        })).await?;
        if let Some(existing) = data["issueLabels"]["nodes"].as_array()
            .and_then(|arr| arr.first())
        {
            return Ok(existing["id"].as_str().unwrap_or("").to_string());
        }
        // Create
        let mq = r#"
            mutation($input: IssueLabelCreateInput!) {
              issueLabelCreate(input: $input) { success issueLabel { id } }
            }
        "#;
        let mdata: serde_json::Value = self.graphql(mq, serde_json::json!({
            "input": { "teamId": team_id, "name": name, "color": color }
        })).await?;
        Ok(mdata["issueLabelCreate"]["issueLabel"]["id"].as_str().unwrap_or("").to_string())
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

pub async fn list_teams(token: &str) -> Result<Vec<LinearTeam>> {
    LinearClient::new(token).list_teams().await
}

pub async fn list_projects(token: &str, team_id: &str) -> Result<Vec<LinearProject>> {
    LinearClient::new(token).list_projects(team_id).await
}

pub async fn list_workflow_states(token: &str, team_id: &str) -> Result<Vec<LinearWorkflowState>> {
    LinearClient::new(token).list_workflow_states(team_id).await
}

/// Setup columns in the Monotask board to match Linear workflow states.
/// Returns (done_column_id, done_state_id) — creates a Done column if needed.
pub async fn setup_columns_from_states(
    doc: &mut AutoCommit,
    token: &str,
    team_id: &str,
    preferred_done_col: Option<&str>,
) -> Result<(String, String)> {
    let states = list_workflow_states(token, team_id).await?;
    let existing_cols = monotask_core::column::list_columns(doc)?;
    let existing_by_name: HashMap<String, String> = existing_cols.iter()
        .map(|c| (c.title.to_lowercase(), c.id.clone()))
        .collect();

    // Separate done vs active states
    let done_states: Vec<&LinearWorkflowState> = states.iter()
        .filter(|s| is_done_state(&s.type_, &s.name))
        .collect();
    let active_states: Vec<&LinearWorkflowState> = states.iter()
        .filter(|s| !is_done_state(&s.type_, &s.name))
        .collect();

    // Create missing active-state columns
    for state in &active_states {
        if !existing_by_name.contains_key(&state.name.to_lowercase()) {
            monotask_core::column::create_column(doc, &state.name)
                .context(format!("create column {}", state.name))?;
        }
    }

    // Determine done column
    let done_col_id = if let Some(col_id) = preferred_done_col {
        col_id.to_string()
    } else {
        // Try to find existing column matching a done state name
        let matched = done_states.iter().find_map(|s| {
            existing_by_name.get(&s.name.to_lowercase()).cloned()
        });
        if let Some(id) = matched {
            id
        } else {
            // Re-read columns after creation above, then find or create Done
            let cols = monotask_core::column::list_columns(doc)?;
            let done_name = done_states.first().map(|s| s.name.as_str()).unwrap_or("Done");
            let found = cols.iter().find(|c| c.title.to_lowercase() == done_name.to_lowercase());
            if let Some(c) = found {
                c.id.clone()
            } else {
                monotask_core::column::create_column(doc, done_name)?
            }
        }
    };

    // Get the first "completed" state id for pushing done cards back to Linear
    let done_state_id = done_states.iter()
        .find(|s| s.type_ == "completed")
        .map(|s| s.id.clone())
        .or_else(|| done_states.first().map(|s| s.id.clone()))
        .unwrap_or_default();

    Ok((done_col_id, done_state_id))
}

// ── Main sync ─────────────────────────────────────────────────────────────────

pub async fn sync_board(
    doc: &mut AutoCommit,
    token: &str,
    config: &LinearConfig,
    actor_pk: &[u8],
) -> Result<SyncResult> {
    let mut result = SyncResult { pulled: 0, pushed: 0, closed: 0, errors: vec![] };
    let client = LinearClient::new(token);

    let sync_start_ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // Fetch workflow states for name-based column mapping
    let states = client.list_workflow_states(&config.team_id).await
        .context("fetching workflow states")?;
    let _state_id_to_name: HashMap<String, String> = states.iter()
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect();
    let state_name_to_id: HashMap<String, String> = states.iter()
        .map(|s| (s.name.to_lowercase(), s.id.clone()))
        .collect();

    // Fetch all issues for this project (incremental by last_sync)
    let since = config.last_sync.as_deref()
        .and_then(|s| if s.is_empty() { None } else { Some(s) });
    let issues = client.list_issues(&config.project_id, since).await
        .context("fetching Linear issues")?;
    let issue_map: HashMap<String, &LinearIssue> = issues.iter().map(|i| (i.id.clone(), i)).collect();

    // Build column maps
    let columns = monotask_core::column::list_columns(doc).context("listing columns")?;
    let col_title_to_id: HashMap<String, String> = columns.iter()
        .map(|c| (c.title.to_lowercase(), c.id.clone())).collect();
    let col_id_to_title: HashMap<String, String> = columns.iter()
        .map(|c| (c.id.clone(), c.title.clone())).collect();

    // Build card_id → column_id map
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

    // Build actor members list
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

    let cards_map_obj = monotask_core::get_cards_map_readonly(doc).context("getting cards map")?;
    let all_card_ids: Vec<String> = doc.keys(&cards_map_obj).map(|k| k.to_string()).collect();

    // Build linear_issue_id → card_id map
    let mut issue_id_to_card: HashMap<String, String> = HashMap::new();
    for card_id in &all_card_ids {
        if let Some(issue_id) = get_linear_issue_id(doc, card_id) {
            issue_id_to_card.insert(issue_id, card_id.clone());
        }
    }

    // ── Phase 1: Pull Linear → CRDT ───────────────────────────────────────────
    for issue in &issues {
        let state_is_done = is_done_state(&issue.state.type_, &issue.state.name);
        let target_col_id = if state_is_done {
            config.done_column_id.clone()
        } else {
            col_title_to_id.get(&issue.state.name.to_lowercase())
                .cloned()
                .unwrap_or_else(|| columns.first().map(|c| c.id.clone()).unwrap_or_default())
        };

        if let Some(card_id) = issue_id_to_card.get(&issue.id).cloned() {
            let is_dead = monotask_core::card::is_tombstoned(doc, &card_id).unwrap_or(true);
            if is_dead { continue; }

            let synced_at = get_linear_synced_at(doc, &card_id);
            let needs_pull = synced_at.as_deref()
                .map(|local_ts| issue.updated_at.as_str() > local_ts)
                .unwrap_or(true);

            if needs_pull {
                if let Err(e) = monotask_core::card::rename_card(doc, &card_id, &issue.title) {
                    result.errors.push(format!("rename_card {card_id}: {e}")); continue;
                }

                let desc_raw = issue.description.as_deref().unwrap_or("");
                let assignee_name = issue.assignee.as_ref().map(|a| a.name.as_str());
                let desc = build_description_with_assignee(desc_raw, assignee_name);
                let _ = monotask_core::card::set_description(doc, &card_id, &desc);

                // Priority → impact/effort
                if let Some((impact, effort)) = linear_priority_to_impact_effort(issue.priority) {
                    let _ = monotask_core::card::set_impact(doc, &card_id, impact);
                    let _ = monotask_core::card::set_effort(doc, &card_id, effort);
                }

                // Move card if state/column changed
                let current_col = card_to_col.get(&card_id).cloned();
                if let Some(from) = current_col {
                    if from != target_col_id {
                        if let Err(e) = monotask_core::column::move_card(doc, &card_id, &from, &target_col_id) {
                            result.errors.push(format!("move_card {card_id}: {e}"));
                        } else {
                            card_to_col.insert(card_id.clone(), target_col_id.clone());
                        }
                    }
                }

                // Sync labels (non-state labels)
                sync_labels(doc, &card_id, &issue.labels.nodes, &col_title_to_id);

                let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
                let _ = set_linear_synced_at(doc, &card_id, &now);
                result.pulled += 1;
            }
        } else {
            // No linked card — try to match by title to avoid duplicates on first sync
            let title_match = all_card_ids.iter().find(|cid| {
                if get_linear_issue_id(doc, cid).is_some() { return false; }
                if monotask_core::card::is_tombstoned(doc, cid).unwrap_or(true) { return false; }
                monotask_core::card::read_card(doc, cid)
                    .map(|c| c.title == issue.title)
                    .unwrap_or(false)
            }).cloned();

            let (linked_card_id, is_new) = if let Some(existing) = title_match {
                (existing, false)
            } else {
                match monotask_core::card::create_card(doc, &target_col_id, &issue.title, actor_pk, &all_members) {
                    Ok(card) => { card_to_col.insert(card.id.clone(), target_col_id.clone()); (card.id, true) }
                    Err(e) => { result.errors.push(format!("create_card for issue {}: {e}", issue.identifier)); continue; }
                }
            };

            let desc_raw = issue.description.as_deref().unwrap_or("");
            let assignee_name = issue.assignee.as_ref().map(|a| a.name.as_str());
            let desc = build_description_with_assignee(desc_raw, assignee_name);
            if !desc.is_empty() {
                let _ = monotask_core::card::set_description(doc, &linked_card_id, &desc);
            }
            let _ = set_linear_issue_id(doc, &linked_card_id, &issue.id);
            let _ = set_linear_issue_identifier(doc, &linked_card_id, &issue.identifier);

            if let Some((impact, effort)) = linear_priority_to_impact_effort(issue.priority) {
                let _ = monotask_core::card::set_impact(doc, &linked_card_id, impact);
                let _ = monotask_core::card::set_effort(doc, &linked_card_id, effort);
            }

            let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let _ = set_linear_synced_at(doc, &linked_card_id, &now);
            sync_labels(doc, &linked_card_id, &issue.labels.nodes, &col_title_to_id);

            if !is_new {
                if let Some(from) = card_to_col.get(&linked_card_id).cloned() {
                    if from != target_col_id {
                        let _ = monotask_core::column::move_card(doc, &linked_card_id, &from, &target_col_id);
                        card_to_col.insert(linked_card_id.clone(), target_col_id.clone());
                    }
                }
            }

            issue_id_to_card.insert(issue.id.clone(), linked_card_id);
            result.pulled += 1;
        }
    }

    // ── Phase 1b: Pull Linear comments → CRDT ────────────────────────────────
    for issue in &issues {
        let card_id = match issue_id_to_card.get(&issue.id) {
            Some(id) => id.clone(),
            None => continue,
        };
        if monotask_core::card::is_tombstoned(doc, &card_id).unwrap_or(true) { continue; }

        let already_imported = card_linear_comment_ids(doc, &card_id);

        for comment in &issue.comments.nodes {
            if already_imported.contains_key(&comment.id) { continue; }

            let author = comment.user.as_ref().map(|u| u.name.as_str()).unwrap_or("Linear user");
            let author_label = format!("@{}", author);
            let text = format!("{}: {}", author_label, comment.body);

            match monotask_core::comment::add_comment(doc, &card_id, &text, &author_label) {
                Ok(local) => {
                    set_comment_linear_id(doc, &card_id, &local.id, &comment.id);
                    result.pulled += 1;
                }
                Err(e) => result.errors.push(format!(
                    "add_comment card={card_id} linear_comment={}: {e}", comment.id
                )),
            }
        }
    }

    // ── Phase 2: Push CRDT → Linear ───────────────────────────────────────────
    // Ensure labels exist on Linear for each non-done column
    let label_colors = ["0075ca", "e4e669", "d93f0b", "0e8a16", "1d76db", "5319e7", "b60205", "f9d0c4"];
    for (i, col) in columns.iter().enumerate() {
        if col.id != config.done_column_id {
            let color = format!("#{}", label_colors[i % label_colors.len()]);
            if let Err(e) = client.ensure_label_exists(&config.team_id, &col.title, &color).await {
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

        // Determine target Linear state
        let state_id = if is_done {
            config.done_state_id.clone()
        } else {
            let col_title = col_id_to_title.get(&col_id).map(|s| s.to_lowercase()).unwrap_or_default();
            state_name_to_id.get(&col_title).cloned().unwrap_or_default()
        };

        // Build labels: column label + card labels
        let mut issue_labels: Vec<String> = card.labels.clone();
        if !is_done {
            if let Some(col_title) = col_id_to_title.get(&col_id) {
                if !issue_labels.contains(col_title) {
                    issue_labels.insert(0, col_title.clone());
                }
            }
        }

        // Resolve label IDs for Linear
        let label_ids = resolve_label_ids(&client, &config.team_id, &issue_labels, &mut result).await;

        // Build priority
        let priority = match (card.impact, card.effort) {
            (Some(imp), Some(eff)) => compute_priority_to_linear(imp, eff),
            _ => 0,
        };

        // Strip assignee footer from description before pushing
        let desc = strip_assignee_footer(&card.description);

        if let Some(issue_id) = get_linear_issue_id(doc, card_id) {
            if let Some(&issue) = issue_map.get(&issue_id) {
                // Compare to detect if push needed
                let current_state = if is_done_state(&issue.state.type_, &issue.state.name) {
                    config.done_column_id.clone()
                } else {
                    col_title_to_id.get(&issue.state.name.to_lowercase()).cloned().unwrap_or_default()
                };
                let remote_desc = strip_assignee_footer(issue.description.as_deref().unwrap_or(""));
                let needs_push = issue.title != card.title
                    || remote_desc != desc
                    || current_state != col_id
                    || issue.priority != priority;

                if needs_push {
                    let mut input = serde_json::json!({
                        "title": card.title,
                        "description": desc,
                        "priority": priority,
                    });
                    if !state_id.is_empty() {
                        input["stateId"] = serde_json::json!(state_id);
                    }
                    if !label_ids.is_empty() {
                        input["labelIds"] = serde_json::json!(label_ids);
                    }
                    if let Err(e) = client.update_issue(&issue_id, &input).await {
                        result.errors.push(format!("update_issue {issue_id}: {e}"));
                    } else {
                        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
                        let _ = set_linear_synced_at(doc, card_id, &now);
                        result.pushed += 1;
                    }
                }
            }
        } else {
            // Create new Linear issue for this card
            let mut input = serde_json::json!({
                "title": card.title,
                "description": desc,
                "teamId": config.team_id,
                "projectId": config.project_id,
                "priority": priority,
            });
            if !state_id.is_empty() {
                input["stateId"] = serde_json::json!(state_id);
            }
            if !label_ids.is_empty() {
                input["labelIds"] = serde_json::json!(label_ids);
            }

            match client.create_issue(&input).await {
                Ok((new_id, identifier)) => {
                    if let Err(e) = set_linear_issue_id(doc, card_id, &new_id) {
                        result.errors.push(format!("set_linear_issue_id {card_id}: {e}"));
                    } else {
                        let _ = set_linear_issue_identifier(doc, card_id, &identifier);
                        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
                        let _ = set_linear_synced_at(doc, card_id, &now);
                        issue_id_to_card.insert(new_id.clone(), card_id.clone());
                        result.pushed += 1;
                    }
                }
                Err(e) => result.errors.push(format!("create_issue for {card_id}: {e}")),
            }
        }
    }

    // ── Phase 2b: Push new local comments → Linear ────────────────────────────
    for card_id in &all_card_ids {
        if monotask_core::card::is_tombstoned(doc, card_id).unwrap_or(true) { continue; }
        let issue_id = match get_linear_issue_id(doc, card_id) {
            Some(id) => id,
            None => continue,
        };

        let local_comments = match monotask_core::comment::list_comments(doc, card_id) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let already_pushed = card_local_comment_linear_ids(doc, card_id);

        for comment in &local_comments {
            if already_pushed.contains_key(&comment.id) { continue; }
            // Skip comments imported from Linear (they start with "@Name: ")
            // Only push locally-created comments (those without a linear_comment_id)
            match client.create_comment(&issue_id, &comment.text).await {
                Ok(linear_id) => {
                    set_comment_linear_id(doc, card_id, &comment.id, &linear_id);
                    result.pushed += 1;
                }
                Err(e) => result.errors.push(format!(
                    "create_comment issue={issue_id} comment={}: {e}", comment.id
                )),
            }
        }
    }

    // ── Phase 3: Cancel deleted cards' issues ─────────────────────────────────
    for card_id in &all_card_ids {
        let is_dead = monotask_core::card::is_tombstoned(doc, card_id).unwrap_or(false);
        if !is_dead { continue; }
        if let Some(issue_id) = get_linear_issue_id(doc, card_id) {
            if let Some(&issue) = issue_map.get(&issue_id) {
                if !is_done_state(&issue.state.type_, &issue.state.name) {
                    let input = serde_json::json!({ "stateId": config.done_state_id });
                    if let Err(e) = client.update_issue(&issue_id, &input).await {
                        result.errors.push(format!("cancel_issue {issue_id}: {e}"));
                    } else {
                        result.closed += 1;
                    }
                }
            }
        }
    }

    // Update last_sync
    let updated = LinearConfig { last_sync: Some(sync_start_ts), ..config.clone() };
    let _ = set_linear_config(doc, Some(&updated));

    Ok(result)
}

pub async fn sync_single_linear_card(
    doc: &mut AutoCommit,
    token: &str,
    config: &LinearConfig,
    card_id: &str,
    _actor_pk: &[u8],
) -> Result<SyncResult> {
    let mut result = SyncResult { pulled: 0, pushed: 0, closed: 0, errors: vec![] };
    let client = LinearClient::new(token);

    let issue_id = get_linear_issue_id(doc, card_id)
        .ok_or_else(|| anyhow::anyhow!("Card not linked to a Linear issue"))?;

    let issue = client.get_issue(&issue_id).await.context("fetching Linear issue")?;

    let states = client.list_workflow_states(&config.team_id).await
        .context("fetching workflow states")?;
    let state_name_to_id: HashMap<String, String> = states.iter()
        .map(|s| (s.name.to_lowercase(), s.id.clone()))
        .collect();

    let columns = monotask_core::column::list_columns(doc).context("listing columns")?;
    let col_title_to_id: HashMap<String, String> = columns.iter()
        .map(|c| (c.title.to_lowercase(), c.id.clone())).collect();
    let col_id_to_title: HashMap<String, String> = columns.iter()
        .map(|c| (c.id.clone(), c.title.clone())).collect();

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

    // ── Pull Linear → CRDT ────────────────────────────────────────────────────
    let state_is_done = is_done_state(&issue.state.type_, &issue.state.name);
    let target_col_id = if state_is_done {
        config.done_column_id.clone()
    } else {
        col_title_to_id.get(&issue.state.name.to_lowercase())
            .cloned()
            .unwrap_or_else(|| columns.first().map(|c| c.id.clone()).unwrap_or_default())
    };

    let is_dead = monotask_core::card::is_tombstoned(doc, card_id).unwrap_or(true);
    if !is_dead {
        if let Err(e) = monotask_core::card::rename_card(doc, card_id, &issue.title) {
            result.errors.push(format!("rename_card: {e}"));
        } else {
            let desc_raw = issue.description.as_deref().unwrap_or("");
            let assignee_name = issue.assignee.as_ref().map(|a| a.name.as_str());
            let desc = build_description_with_assignee(desc_raw, assignee_name);
            let _ = monotask_core::card::set_description(doc, card_id, &desc);

            if let Some((impact, effort)) = linear_priority_to_impact_effort(issue.priority) {
                let _ = monotask_core::card::set_impact(doc, card_id, impact);
                let _ = monotask_core::card::set_effort(doc, card_id, effort);
            }

            if let Some(from) = card_to_col.get(card_id).cloned() {
                if from != target_col_id {
                    if let Err(e) = monotask_core::column::move_card(doc, card_id, &from, &target_col_id) {
                        result.errors.push(format!("move_card: {e}"));
                    } else {
                        card_to_col.insert(card_id.to_string(), target_col_id.clone());
                    }
                }
            }

            sync_labels(doc, card_id, &issue.labels.nodes, &col_title_to_id);
            let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let _ = set_linear_synced_at(doc, card_id, &now);
            result.pulled += 1;
        }

        // Pull comments
        let already_imported = card_linear_comment_ids(doc, card_id);
        for comment in &issue.comments.nodes {
            if already_imported.contains_key(&comment.id) { continue; }
            let author = comment.user.as_ref().map(|u| u.name.as_str()).unwrap_or("Linear user");
            let text = format!("@{}: {}", author, comment.body);
            match monotask_core::comment::add_comment(doc, card_id, &text, &format!("@{author}")) {
                Ok(local) => {
                    set_comment_linear_id(doc, card_id, &local.id, &comment.id);
                    result.pulled += 1;
                }
                Err(e) => result.errors.push(format!("add_comment: {e}")),
            }
        }
    }

    // ── Push CRDT → Linear ────────────────────────────────────────────────────
    if !is_dead {
        let card = monotask_core::card::read_card(doc, card_id)
            .map_err(|e| anyhow::anyhow!("read_card: {e}"))?;

        let col_id = card_to_col.get(card_id).cloned().unwrap_or_default();
        let is_done = col_id == config.done_column_id;

        let state_id = if is_done {
            config.done_state_id.clone()
        } else {
            let col_title = col_id_to_title.get(&col_id).map(|s| s.to_lowercase()).unwrap_or_default();
            state_name_to_id.get(&col_title).cloned().unwrap_or_default()
        };

        let mut issue_labels: Vec<String> = card.labels.clone();
        if !is_done {
            if let Some(col_title) = col_id_to_title.get(&col_id) {
                if !issue_labels.contains(col_title) {
                    issue_labels.insert(0, col_title.clone());
                }
            }
        }
        let label_ids = resolve_label_ids(&client, &config.team_id, &issue_labels, &mut result).await;

        let priority = match (card.impact, card.effort) {
            (Some(imp), Some(eff)) => compute_priority_to_linear(imp, eff),
            _ => 0,
        };
        let desc = strip_assignee_footer(&card.description);
        let remote_desc = strip_assignee_footer(issue.description.as_deref().unwrap_or(""));
        let needs_push = issue.title != card.title
            || remote_desc != desc
            || target_col_id != col_id
            || issue.priority != priority;

        if needs_push {
            let mut input = serde_json::json!({
                "title": card.title,
                "description": desc,
                "priority": priority,
            });
            if !state_id.is_empty() { input["stateId"] = serde_json::json!(state_id); }
            if !label_ids.is_empty() { input["labelIds"] = serde_json::json!(label_ids); }
            if let Err(e) = client.update_issue(&issue_id, &input).await {
                result.errors.push(format!("update_issue: {e}"));
            } else {
                let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
                let _ = set_linear_synced_at(doc, card_id, &now);
                result.pushed += 1;
            }
        }

        // Push new local comments
        let local_comments = monotask_core::comment::list_comments(doc, card_id).unwrap_or_default();
        let already_pushed = card_local_comment_linear_ids(doc, card_id);
        for comment in &local_comments {
            if already_pushed.contains_key(&comment.id) { continue; }
            match client.create_comment(&issue_id, &comment.text).await {
                Ok(linear_id) => {
                    set_comment_linear_id(doc, card_id, &comment.id, &linear_id);
                    result.pushed += 1;
                }
                Err(e) => result.errors.push(format!("create_comment: {e}")),
            }
        }
    }

    Ok(result)
}

// ── Helpers for sync ──────────────────────────────────────────────────────────

fn sync_labels(
    doc: &mut AutoCommit,
    card_id: &str,
    linear_labels: &[LinearLabel],
    col_title_to_id: &HashMap<String, String>,
) {
    let non_col: std::collections::HashSet<String> = linear_labels.iter()
        .filter(|l| !col_title_to_id.contains_key(&l.name.to_lowercase()))
        .map(|l| l.name.clone())
        .collect();
    let current = monotask_core::card::read_card(doc, card_id)
        .map(|c| c.labels)
        .unwrap_or_default();
    for lbl in &current {
        if !col_title_to_id.contains_key(lbl.to_lowercase().as_str()) && !non_col.contains(lbl) {
            let _ = monotask_core::card::remove_label(doc, card_id, lbl);
        }
    }
    for lbl in &non_col {
        let _ = monotask_core::card::add_label(doc, card_id, lbl);
    }
}

async fn resolve_label_ids(
    client: &LinearClient,
    team_id: &str,
    label_names: &[String],
    result: &mut SyncResult,
) -> Vec<String> {
    let mut ids = Vec::new();
    for name in label_names {
        let color = "#0075ca";
        match client.ensure_label_exists(team_id, name, color).await {
            Ok(id) if !id.is_empty() => ids.push(id),
            Ok(_) => {}
            Err(e) => result.errors.push(format!("resolve_label '{name}': {e}")),
        }
    }
    ids
}
