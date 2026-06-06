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
//! Auth: `az login` alone is enough — we mint a bearer token via `az account
//! get-access-token`. Alternatively set a PAT in `AZURE_DEVOPS_EXT_PAT` (the
//! conventional var; `AZURE_DEVOPS_PAT` / `ADO_PAT` also work), sent via HTTP
//! Basic. Only core `az` (or a PAT) is required — not the `azure-devops`
//! extension.

use std::env;
use std::process::Command;
use std::sync::Arc;

use reqwest::header::ACCEPT;
use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::command::diff::git::{build_file_diff, percent_encode};
use crate::command::diff::pr_provider::{PrError, ProviderData};
use crate::command::diff::types::FileDiff;
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

    /// GET `url` and deserialize the JSON body into `T`. Unknown fields are
    /// ignored, so each caller's struct declares only the fields it needs.
    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, PrError> {
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
        serde_json::from_str(&body)
            .map_err(|e| PrError::Other(format!("invalid JSON from Azure: {}", e)))
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
            self.git_url(&format!(
                "repositories/{}/blobs/{}",
                percent_encode(&self.repo),
                blob_id
            )),
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
        let list: IterationList = self.get(&url).await?;
        list.value
            .iter()
            .map(|i| i.id)
            .max()
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
            let page: ChangesPage = self.get(&url).await?;
            let page_len = page.change_entries.len();
            entries.extend(
                page.change_entries
                    .into_iter()
                    .filter_map(RawChange::into_change),
            );
            // `nextSkip` is the canonical "more pages" signal; fall back to a
            // short page meaning we're done.
            if page.next_skip == 0 || page_len < CHANGES_PAGE {
                break;
            }
            skip = page.next_skip as usize;
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
                let _permit = sem
                    .acquire_owned()
                    .await
                    .expect("blob semaphore not closed");
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

/// The PR iterations list; we only need each iteration's numeric id.
#[derive(Deserialize)]
struct IterationList {
    value: Vec<Iteration>,
}

#[derive(Deserialize)]
struct Iteration {
    id: u64,
}

/// A page of the iteration-changes endpoint.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangesPage {
    #[serde(default)]
    change_entries: Vec<RawChange>,
    /// Canonical "more pages" cursor; `0`/absent means we're done.
    #[serde(default)]
    next_skip: u64,
}

/// A `changeEntries[]` entry exactly as Azure returns it.
#[derive(Deserialize)]
struct RawChange {
    item: Option<RawItem>,
    #[serde(rename = "originalPath")]
    original_path: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawItem {
    path: Option<String>,
    object_id: Option<String>,
    original_object_id: Option<String>,
    #[serde(default)]
    is_folder: bool,
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

impl RawChange {
    /// Reduce a wire entry to a [`ChangeEntry`], skipping folders and entries
    /// with no path. The rename case falls back to `originalPath`.
    fn into_change(self) -> Option<ChangeEntry> {
        let item = self.item.unwrap_or_default();
        if item.is_folder {
            return None;
        }
        let raw_path = item.path.or(self.original_path)?;
        Some(ChangeEntry {
            path: raw_path.trim_start_matches('/').to_string(),
            new_blob: item.object_id.filter(|s| !s.is_empty()),
            old_blob: item.original_object_id.filter(|s| !s.is_empty()),
        })
    }
}

/// The PR detail endpoint, reduced to the refs and repo name we display.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrDetail {
    #[serde(default)]
    source_ref_name: String,
    #[serde(default)]
    target_ref_name: String,
    repository: Option<RepoRef>,
}

#[derive(Deserialize)]
struct RepoRef {
    name: Option<String>,
}

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
        let detail: PrDetail = client.get(&url).await?;
        Ok(AzurePrMeta {
            source_ref: detail.source_ref_name,
            target_ref: detail.target_ref_name,
            repo_name: detail
                .repository
                .and_then(|r| r.name)
                .unwrap_or_else(|| repo.to_string()),
        })
    })
}

/// The active-PR search result; we take the first match's id.
#[derive(Deserialize)]
struct PrList {
    value: Vec<PrId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrId {
    pull_request_id: u64,
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
            client.git_url(&format!(
                "repositories/{}/pullrequests",
                percent_encode(repo)
            )),
            percent_encode(&format!("refs/heads/{}", branch)),
            API_VERSION
        );
        let list: PrList = client.get(&url).await?;
        list.value
            .first()
            .map(|pr| pr.pull_request_id)
            .ok_or_else(|| PrError::NotFound(format!("No active PR found for branch {}", branch)))
    })
}

pub fn load_pr_file_diffs(pr: &PrInfo) -> Result<Vec<FileDiff>, PrError> {
    let ProviderData::Azure { org_url, project } = &pr.data else {
        return Err(PrError::Other(
            "Azure PR missing organisation/project data".to_string(),
        ));
    };
    let client = AdoClient::new(org_url, project, &pr.repo_name)?;
    let pr_id = pr.number;
    block_on(async move { client.load_file_diffs(pr_id).await })
}

/// Run an async future to completion from the synchronous diff path. `main` is
/// a multi-threaded `#[tokio::main]`, so we mark the current worker as blocking
/// and drive the future on the existing runtime — no second runtime, no new deps.
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(fut))
}

fn resolve_auth() -> Result<AdoAuth, PrError> {
    // `AZURE_DEVOPS_EXT_PAT` is the conventional Azure DevOps PAT var (read by
    // the `az devops` CLI extension); the others are accepted as aliases.
    for var in ["AZURE_DEVOPS_EXT_PAT", "AZURE_DEVOPS_PAT", "ADO_PAT"] {
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
                "No Azure DevOps credentials: run `az login`, or set AZURE_DEVOPS_EXT_PAT ({})",
                e
            ))
        })?;
    if !output.status.success() {
        return Err(PrError::Auth(
            "No Azure DevOps credentials: run `az login`, or set AZURE_DEVOPS_EXT_PAT.".to_string(),
        ));
    }
    let token: TokenResponse = serde_json::from_slice(&output.stdout)
        .map_err(|e| PrError::Other(format!("Could not parse az token output: {}", e)))?;
    token
        .access_token
        .map(AdoAuth::Bearer)
        .ok_or_else(|| PrError::Auth("az returned no access token".to_string()))
}

/// The `az account get-access-token --output json` response.
#[derive(Deserialize)]
struct TokenResponse {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
}

fn auth_hint(status: reqwest::StatusCode, body: &str) -> PrError {
    use reqwest::StatusCode;
    match status {
        StatusCode::UNAUTHORIZED => PrError::Auth(
            "Azure DevOps auth failed (401). Check your PAT scopes (Code: Read) or run `az login`."
                .to_string(),
        ),
        StatusCode::FORBIDDEN => PrError::Auth(
            "Azure DevOps returned 403. The token lacks access to this repository.".to_string(),
        ),
        StatusCode::NOT_FOUND => {
            PrError::NotFound("Azure DevOps returned 404 (PR or repository not found).".to_string())
        }
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

    /// Deserialize a wire change entry and reduce it the way `changes()` does.
    fn change(v: serde_json::Value) -> Option<ChangeEntry> {
        serde_json::from_value::<RawChange>(v)
            .unwrap()
            .into_change()
    }

    #[test]
    fn parses_add_edit_delete_changes() {
        let add = change(serde_json::json!({
            "changeType": "add",
            "item": { "path": "/src/new.rs", "objectId": "newsha" }
        }))
        .unwrap();
        assert_eq!(add.path, "src/new.rs");
        assert_eq!(add.new_blob.as_deref(), Some("newsha"));
        assert_eq!(add.old_blob, None);

        let edit = change(serde_json::json!({
            "changeType": "edit",
            "item": { "path": "/a.txt", "objectId": "n", "originalObjectId": "o" }
        }))
        .unwrap();
        assert_eq!(edit.new_blob.as_deref(), Some("n"));
        assert_eq!(edit.old_blob.as_deref(), Some("o"));

        let del = change(serde_json::json!({
            "changeType": "delete",
            "item": { "path": "/gone.rs", "originalObjectId": "o" }
        }))
        .unwrap();
        assert_eq!(del.new_blob, None);
        assert_eq!(del.old_blob.as_deref(), Some("o"));
    }

    #[test]
    fn skips_folders() {
        assert!(change(serde_json::json!({
            "changeType": "add",
            "item": { "path": "/dir", "isFolder": true }
        }))
        .is_none());
    }

    #[test]
    fn falls_back_to_original_path_on_rename() {
        let renamed = change(serde_json::json!({
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
        assert_eq!(
            percent_encode("refs/heads/feature/x"),
            "refs%2Fheads%2Ffeature%2Fx"
        );
        assert_eq!(percent_encode("simple-repo.git"), "simple-repo.git");
    }
}
