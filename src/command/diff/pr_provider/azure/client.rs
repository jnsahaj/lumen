//! Azure DevOps REST client for pull-request review.
//!
//! The `az` CLI has no first-class command to list a PR's changed files or
//! fetch file content, and the generic `az devops invoke` passthrough spawns a
//! Python process per call — unusable for an interactive diff. So we talk to the
//! Azure DevOps REST API directly.
//!
//! Diffs use the iteration-changes endpoint with `$compareTo=0`, i.e. the diff
//! against the PR's merge base — the same three-dot view the Azure web UI shows,
//! rather than a raw tip-to-tip comparison.
//!
//! Auth resolves to a PAT (`ADO_PAT` / `AZURE_DEVOPS_EXT_PAT`) via HTTP Basic,
//! or falls back to a bearer token from `az account get-access-token`. Only core
//! `az` (or a PAT) is required — not the `azure-devops` extension.

use std::env;
use std::process::Command;
use std::sync::Arc;

use reqwest::header::ACCEPT;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::command::diff::git::{build_file_diff, percent_encode};
use crate::command::diff::types::FileDiff;
use crate::command::diff::pr_provider::PrError;
use crate::command::diff::PrInfo;

const API_VERSION: &str = "7.1";
/// Azure DevOps OAuth resource id, used with `az account get-access-token`.
const ADO_RESOURCE: &str = "499b84ac-1321-427f-aa17-267ca6975798";
/// Max concurrent blob fetches — keeps us well under Azure's rate limits while
/// still parallelising content retrieval.
const BLOB_CONCURRENCY: usize = 8;
/// Page size for the paginated iteration-changes endpoint.
const CHANGES_PAGE: usize = 1000;

/// PR metadata needed to drive the diff UI.
pub struct AzurePrMeta {
    pub source_ref: String,
    pub target_ref: String,
    pub repo_name: String,
}

enum AdoAuth {
    /// Personal access token, sent via HTTP Basic with an empty username.
    Pat(String),
    /// OAuth bearer token (from `az account get-access-token`).
    Bearer(String),
}

#[derive(Clone)]
struct AdoClient {
    http: reqwest::Client,
    /// Organisation base URL, e.g. `https://dev.azure.com/org`.
    base: String,
    project: String,
    repo: String,
    auth: Arc<AdoAuth>,
}

impl AdoClient {
    fn new(org_url: &str, project: &str, repo: &str) -> Result<Self, PrError> {
        Ok(Self {
            http: reqwest::Client::new(),
            base: org_url.trim_end_matches('/').to_string(),
            project: project.to_string(),
            repo: repo.to_string(),
            auth: Arc::new(resolve_auth()?),
        })
    }

    fn authed(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.auth.as_ref() {
            AdoAuth::Pat(pat) => rb.basic_auth("", Some(pat)),
            AdoAuth::Bearer(token) => rb.bearer_auth(token),
        }
    }

    /// `{base}/{project}/_apis/git/...` with project URL-encoded.
    fn git_url(&self, suffix: &str) -> String {
        format!(
            "{}/{}/_apis/git/{}",
            self.base,
            percent_encode(&self.project),
            suffix
        )
    }

    async fn get_json(&self, url: &str) -> Result<Value, PrError> {
        let resp = self
            .authed(self.http.get(url))
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(auth_hint(status, &body));
        }
        serde_json::from_str(&body).map_err(|e| PrError::Other(format!("invalid JSON from Azure: {}", e)))
    }

    /// Fetch a blob's text content. An absent `blob_id` (the missing side of an
    /// add/delete) is `Ok("")`; a failed fetch is an `Err` so it can't silently
    /// empty a side and flip the file's status to Added/Deleted downstream.
    async fn blob_text(&self, blob_id: Option<&str>) -> Result<String, PrError> {
        let Some(blob_id) = blob_id else {
            return Ok(String::new());
        };
        let url = format!(
            "{}?$format=text&api-version={}",
            self.git_url(&format!("repositories/{}/blobs/{}", percent_encode(&self.repo), blob_id)),
            API_VERSION
        );
        let resp = self
            .authed(self.http.get(&url))
            .header(ACCEPT, "text/plain")
            .send()
            .await
            .map_err(|e| format!("blob {} request failed: {}", blob_id, e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(auth_hint(status, &body));
        }
        // Read as bytes then lossy-decode so binary blobs degrade gracefully
        // (build_file_diff flags them as binary downstream).
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("blob {} read failed: {}", blob_id, e))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn latest_iteration(&self, pr_id: u64) -> Result<u64, PrError> {
        let url = format!(
            "{}?api-version={}",
            self.git_url(&format!(
                "repositories/{}/pullRequests/{}/iterations",
                percent_encode(&self.repo),
                pr_id
            )),
            API_VERSION
        );
        let json = self.get_json(&url).await?;
        json.get("value")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.iter().filter_map(|i| i.get("id")?.as_u64()).max())
            .ok_or_else(|| PrError::Other("PR has no iterations".to_string()))
    }

    /// All change entries for `iteration`, compared against the merge base
    /// (`$compareTo=0`), following `$skip`/`$top` pagination.
    async fn changes(&self, pr_id: u64, iteration: u64) -> Result<Vec<ChangeEntry>, PrError> {
        let mut entries = Vec::new();
        let mut skip = 0usize;
        loop {
            let url = format!(
                "{}?$compareTo=0&$top={}&$skip={}&api-version={}",
                self.git_url(&format!(
                    "repositories/{}/pullRequests/{}/iterations/{}/changes",
                    percent_encode(&self.repo),
                    pr_id,
                    iteration
                )),
                CHANGES_PAGE,
                skip,
                API_VERSION
            );
            let json = self.get_json(&url).await?;
            let page = json
                .get("changeEntries")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let page_len = page.len();
            for entry in page {
                if let Some(change) = ChangeEntry::from_json(&entry) {
                    entries.push(change);
                }
            }
            // `nextSkip` is the canonical "more pages" signal; fall back to a
            // short page meaning we're done.
            let next_skip = json.get("nextSkip").and_then(|v| v.as_u64()).unwrap_or(0);
            if next_skip == 0 || page_len < CHANGES_PAGE {
                break;
            }
            skip = next_skip as usize;
        }
        Ok(entries)
    }

    async fn load_file_diffs(&self, pr_id: u64) -> Result<Vec<FileDiff>, PrError> {
        let iteration = self.latest_iteration(pr_id).await?;
        let changes = self.changes(pr_id, iteration).await?;

        let sem = Arc::new(Semaphore::new(BLOB_CONCURRENCY));
        let mut set: JoinSet<Result<(usize, FileDiff), PrError>> = JoinSet::new();
        for (idx, change) in changes.into_iter().enumerate() {
            let client = self.clone();
            let sem = Arc::clone(&sem);
            set.spawn(async move {
                let _permit = sem.acquire_owned().await.expect("blob semaphore not closed");
                let old = client.blob_text(change.old_blob.as_deref()).await?;
                let new = client.blob_text(change.new_blob.as_deref()).await?;
                Ok((idx, build_file_diff(change.path, old, new)))
            });
        }

        // Reassemble in the original change order.
        let mut out: Vec<Option<FileDiff>> = Vec::new();
        while let Some(res) = set.join_next().await {
            // Outer `?`: the task panicked. Inner `?`: a blob fetch failed.
            let (idx, diff) = res.map_err(|e| format!("blob fetch task failed: {}", e))??;
            if idx >= out.len() {
                out.resize_with(idx + 1, || None);
            }
            out[idx] = Some(diff);
        }
        Ok(out.into_iter().flatten().collect())
    }
}

/// One PR file change reduced to what the diff UI needs.
struct ChangeEntry {
    /// Repo-relative path without a leading slash.
    path: String,
    /// Blob id of the new (head) side, if any.
    new_blob: Option<String>,
    /// Blob id of the old (base) side, if any.
    old_blob: Option<String>,
}

impl ChangeEntry {
    fn from_json(entry: &Value) -> Option<Self> {
        let item = entry.get("item");
        // Skip folders.
        if item
            .and_then(|i| i.get("isFolder"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return None;
        }
        let raw_path = item
            .and_then(|i| i.get("path"))
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("originalPath").and_then(|v| v.as_str()))?;
        let new_blob = item
            .and_then(|i| i.get("objectId"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let old_blob = item
            .and_then(|i| i.get("originalObjectId"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        Some(Self {
            path: raw_path.trim_start_matches('/').to_string(),
            new_blob,
            old_blob,
        })
    }
}

// ---------------------------------------------------------------------------
// Public sync entry points (bridge the async client onto the sync diff path)
// ---------------------------------------------------------------------------

pub fn fetch_pr_metadata(
    org_url: &str,
    project: &str,
    repo: &str,
    pr_id: u64,
) -> Result<AzurePrMeta, PrError> {
    let client = AdoClient::new(org_url, project, repo)?;
    block_on(async move {
        // PR detail is project-scoped (not repo-scoped) in the REST API.
        let url = format!(
            "{}?api-version={}",
            client.git_url(&format!("pullrequests/{}", pr_id)),
            API_VERSION
        );
        let json = client.get_json(&url).await?;
        let source_ref = json
            .get("sourceRefName")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let target_ref = json
            .get("targetRefName")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let repo_name = json
            .pointer("/repository/name")
            .and_then(|v| v.as_str())
            .unwrap_or(repo)
            .to_string();
        Ok(AzurePrMeta {
            source_ref,
            target_ref,
            repo_name,
        })
    })
}

pub fn detect_active_pr(
    org_url: &str,
    project: &str,
    repo: &str,
    branch: &str,
) -> Result<u64, PrError> {
    let client = AdoClient::new(org_url, project, repo)?;
    block_on(async move {
        let url = format!(
            "{}?searchCriteria.status=active&searchCriteria.sourceRefName={}&api-version={}",
            client.git_url(&format!("repositories/{}/pullrequests", percent_encode(repo))),
            percent_encode(&format!("refs/heads/{}", branch)),
            API_VERSION
        );
        let json = client.get_json(&url).await?;
        json.get("value")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|pr| pr.get("pullRequestId"))
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PrError::NotFound(format!("No active PR found for branch {}", branch)))
    })
}

pub fn load_pr_file_diffs(pr: &PrInfo) -> Result<Vec<FileDiff>, PrError> {
    let org_url = pr
        .org_url
        .as_deref()
        .ok_or("Azure PR missing organisation URL")?;
    let project = pr
        .project
        .as_deref()
        .ok_or("Azure PR missing project")?;
    let client = AdoClient::new(org_url, project, &pr.repo_name)?;
    let pr_id = pr.number;
    block_on(async move { client.load_file_diffs(pr_id).await })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run an async future to completion from the synchronous diff path. `main` is
/// a multi-threaded `#[tokio::main]`, so we mark the current worker as blocking
/// and drive the future on the existing runtime — no second runtime, no new deps.
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(fut))
}

fn resolve_auth() -> Result<AdoAuth, PrError> {
    for var in ["ADO_PAT", "AZURE_DEVOPS_EXT_PAT", "AZURE_DEVOPS_PAT"] {
        if let Ok(pat) = env::var(var) {
            if !pat.trim().is_empty() {
                return Ok(AdoAuth::Pat(pat));
            }
        }
    }
    // Fall back to an OAuth token from the Azure CLI (`az login`).
    let output = Command::new("az")
        .args([
            "account",
            "get-access-token",
            "--resource",
            ADO_RESOURCE,
            "-o",
            "json",
        ])
        .output()
        .map_err(|e| {
            PrError::Auth(format!(
                "No Azure DevOps credentials: set ADO_PAT or install the Azure CLI and run `az login` ({})",
                e
            ))
        })?;
    if !output.status.success() {
        return Err(PrError::Auth(
            "No Azure DevOps credentials: set ADO_PAT, or run `az login` to use the Azure CLI."
                .to_string(),
        ));
    }
    let json: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Could not parse az token output: {}", e))?;
    json.get("accessToken")
        .and_then(|v| v.as_str())
        .map(|t| AdoAuth::Bearer(t.to_string()))
        .ok_or_else(|| PrError::Auth("az returned no access token".to_string()))
}

fn auth_hint(status: reqwest::StatusCode, body: &str) -> PrError {
    use reqwest::StatusCode;
    match status {
        StatusCode::UNAUTHORIZED => PrError::Auth(
            "Azure DevOps auth failed (401). Check ADO_PAT scopes (Code: Read) or run `az login`."
                .to_string(),
        ),
        StatusCode::FORBIDDEN => PrError::Auth(
            "Azure DevOps returned 403. The token lacks access to this repository.".to_string(),
        ),
        StatusCode::NOT_FOUND => PrError::NotFound(
            "Azure DevOps returned 404 (PR or repository not found).".to_string(),
        ),
        _ => {
            let snippet: String = body.chars().take(200).collect();
            PrError::Other(format!(
                "Azure DevOps request failed ({}): {}",
                status,
                snippet.trim()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_add_edit_delete_changes() {
        let add = ChangeEntry::from_json(&serde_json::json!({
            "changeType": "add",
            "item": { "path": "/src/new.rs", "objectId": "newsha" }
        }))
        .unwrap();
        assert_eq!(add.path, "src/new.rs");
        assert_eq!(add.new_blob.as_deref(), Some("newsha"));
        assert_eq!(add.old_blob, None);

        let edit = ChangeEntry::from_json(&serde_json::json!({
            "changeType": "edit",
            "item": { "path": "/a.txt", "objectId": "n", "originalObjectId": "o" }
        }))
        .unwrap();
        assert_eq!(edit.new_blob.as_deref(), Some("n"));
        assert_eq!(edit.old_blob.as_deref(), Some("o"));

        let del = ChangeEntry::from_json(&serde_json::json!({
            "changeType": "delete",
            "item": { "path": "/gone.rs", "originalObjectId": "o" }
        }))
        .unwrap();
        assert_eq!(del.new_blob, None);
        assert_eq!(del.old_blob.as_deref(), Some("o"));
    }

    #[test]
    fn skips_folders() {
        assert!(ChangeEntry::from_json(&serde_json::json!({
            "changeType": "add",
            "item": { "path": "/dir", "isFolder": true }
        }))
        .is_none());
    }

    #[test]
    fn falls_back_to_original_path_on_rename() {
        let renamed = ChangeEntry::from_json(&serde_json::json!({
            "changeType": "rename",
            "item": { "objectId": "n", "originalObjectId": "o" },
            "originalPath": "/old/name.rs"
        }))
        .unwrap();
        assert_eq!(renamed.path, "old/name.rs");
    }

    #[test]
    fn encodes_segments() {
        assert_eq!(percent_encode("My Project"), "My%20Project");
        assert_eq!(percent_encode("refs/heads/feature/x"), "refs%2Fheads%2Ffeature%2Fx");
        assert_eq!(percent_encode("simple-repo.git"), "simple-repo.git");
    }
}
