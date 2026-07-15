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
use std::thread;

use reqwest::header::ACCEPT;
use serde::Deserialize;

use crate::command::diff::git::percent_encode;
use crate::command::diff::pr_provider::PrError;
use crate::command::diff::types::{is_binary_content, FileDiff, FileStatus};

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
    http: reqwest::blocking::Client,
    /// Organisation base URL, e.g. `https://dev.azure.com/org`.
    base: String,
    project: String,
    repo: String,
    auth: Arc<AdoAuth>,
}

impl AdoClient {
    fn new(org_url: &str, project: &str, repo: &str) -> Result<Self, PrError> {
        Ok(Self {
            http: reqwest::blocking::Client::builder()
                .build()
                .map_err(|e| format!("could not create Azure HTTP client: {}", e))?,
            base: org_url.trim_end_matches('/').to_string(),
            project: project.to_string(),
            repo: repo.to_string(),
            auth: Arc::new(resolve_auth()?),
        })
    }

    fn authed(&self, rb: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
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
    fn get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, PrError> {
        let resp = self
            .authed(self.http.get(url))
            .header(ACCEPT, "application/json")
            .send()
            .map_err(|e| format!("request failed: {}", e))?;
        let status = resp.status();
        let body = resp
            .text()
            .map_err(|e| format!("response body read failed: {}", e))?;
        if !status.is_success() {
            return Err(auth_hint(status, &body));
        }
        serde_json::from_str(&body)
            .map_err(|e| PrError::Other(format!("invalid JSON from Azure: {}", e)))
    }

    /// Fetch a blob's text content while retaining whether the side exists.
    fn blob_text(&self, blob_id: Option<&str>) -> Result<Option<String>, PrError> {
        let Some(blob_id) = blob_id else {
            return Ok(None);
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
            .map_err(|e| format!("blob {} request failed: {}", blob_id, e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp
                .text()
                .map_err(|e| format!("blob {} error body read failed: {}", blob_id, e))?;
            return Err(auth_hint(status, &body));
        }
        // Read as bytes then lossy-decode so binary blobs degrade gracefully
        // (build_file_diff flags them as binary downstream).
        let bytes = resp
            .bytes()
            .map_err(|e| format!("blob {} read failed: {}", blob_id, e))?;
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }

    fn latest_iteration(&self, pr_id: u64) -> Result<u64, PrError> {
        let url = format!(
            "{}?api-version={}",
            self.git_url(&format!(
                "repositories/{}/pullRequests/{}/iterations",
                percent_encode(&self.repo),
                pr_id
            )),
            API_VERSION
        );
        let list: IterationList = self.get(&url)?;
        list.value
            .iter()
            .map(|i| i.id)
            .max()
            .ok_or_else(|| PrError::Other("PR has no iterations".to_string()))
    }

    /// All change entries for `iteration`, compared against the merge base
    /// (`$compareTo=0`), following `$skip`/`$top` pagination.
    fn changes(&self, pr_id: u64, iteration: u64) -> Result<Vec<ChangeEntry>, PrError> {
        let mut entries = Vec::new();
        let mut skip = 0_u64;
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
            let page: ChangesPage = self.get(&url)?;
            for raw_change in page.change_entries {
                if let Some(change) = raw_change.into_change()? {
                    entries.push(change);
                }
            }
            match page.next_skip {
                None | Some(0) => break,
                Some(next_skip) if next_skip > skip => skip = next_skip,
                Some(next_skip) => {
                    return Err(PrError::Other(format!(
                        "Azure returned non-advancing nextSkip cursor {} after {}",
                        next_skip, skip
                    )))
                }
            }
        }
        Ok(entries)
    }

    fn fetch_file_diff(&self, change: &ChangeEntry) -> Result<FileDiff, PrError> {
        let old = self.blob_text(change.old_blob.as_deref());
        let new = self.blob_text(change.new_blob.as_deref());
        match (old, new) {
            (Ok(old), Ok(new)) => Ok(build_file_diff(change, old, new)),
            (Err(old), Err(new)) => Err(PrError::Other(format!(
                "both blob fetches failed for {}: {}; {}",
                change.path, old, new
            ))),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }

    fn load_file_diffs(&self, pr_id: u64) -> Result<Vec<FileDiff>, PrError> {
        let iteration = self.latest_iteration(pr_id)?;
        let changes = self.changes(pr_id, iteration)?;
        if changes.is_empty() {
            return Ok(Vec::new());
        }

        let worker_count = changes.len().min(BLOB_CONCURRENCY);
        let base_chunk_size = changes.len() / worker_count;
        let larger_chunks = changes.len() % worker_count;
        let batches = thread::scope(|scope| {
            let mut workers = Vec::with_capacity(worker_count);
            let mut chunk_start = 0;
            for worker_index in 0..worker_count {
                let chunk_len = base_chunk_size + usize::from(worker_index < larger_chunks);
                let chunk = &changes[chunk_start..chunk_start + chunk_len];
                let result_start = chunk_start;
                chunk_start += chunk_len;
                let client = self.clone();
                workers.push(scope.spawn(move || {
                    chunk
                        .iter()
                        .enumerate()
                        .map(|(offset, change)| {
                            (result_start + offset, client.fetch_file_diff(change))
                        })
                        .collect::<Vec<_>>()
                }));
            }

            workers
                .into_iter()
                .map(|worker| worker.join())
                .collect::<Vec<_>>()
        });

        let mut out = std::iter::repeat_with(|| None)
            .take(changes.len())
            .collect::<Vec<Option<FileDiff>>>();
        let mut failures = Vec::new();
        for batch in batches {
            match batch {
                Ok(results) => {
                    for (index, result) in results {
                        match result {
                            Ok(diff) => out[index] = Some(diff),
                            Err(error) => failures.push(error),
                        }
                    }
                }
                Err(_) => failures.push(PrError::Other(
                    "Azure blob fetch worker panicked".to_string(),
                )),
            }
        }

        if failures.len() == 1 {
            return Err(failures.remove(0));
        }
        if !failures.is_empty() {
            return Err(PrError::Other(format!(
                "multiple Azure blob fetch failures: {}",
                failures
                    .into_iter()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        }

        out.into_iter()
            .enumerate()
            .map(|(index, diff)| {
                diff.ok_or_else(|| {
                    PrError::Other(format!(
                        "Azure blob worker returned no result for change {}",
                        index
                    ))
                })
            })
            .collect()
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
    change_entries: Vec<RawChange>,
    /// Canonical "more pages" cursor; `0` or absent means we're done.
    next_skip: Option<u64>,
}

/// A `changeEntries[]` entry exactly as Azure returns it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawChange {
    change_type: AzureChangeType,
    item: Option<RawItem>,
    original_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum AzureChangeType {
    None,
    Add,
    Edit,
    Encoding,
    Rename,
    Delete,
    Undelete,
    Branch,
    Merge,
    Lock,
    Rollback,
    SourceRename,
    TargetRename,
    Property,
    All,
}

impl AzureChangeType {
    fn file_status(self) -> FileStatus {
        match self {
            Self::Add | Self::Undelete => FileStatus::Added,
            Self::Delete => FileStatus::Deleted,
            Self::None
            | Self::Edit
            | Self::Encoding
            | Self::Rename
            | Self::Branch
            | Self::Merge
            | Self::Lock
            | Self::Rollback
            | Self::SourceRename
            | Self::TargetRename
            | Self::Property
            | Self::All => FileStatus::Modified,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawItem {
    path: Option<String>,
    object_id: Option<String>,
    original_object_id: Option<String>,
    #[serde(default)]
    is_folder: bool,
}

/// One PR file change reduced to what the diff UI needs.
#[derive(Debug)]
struct ChangeEntry {
    /// Repo-relative path without a leading slash.
    path: String,
    /// Blob id of the new (head) side, if any.
    new_blob: Option<String>,
    /// Blob id of the old (base) side, if any.
    old_blob: Option<String>,
    status: FileStatus,
}

impl RawChange {
    /// Reduce a wire entry to a [`ChangeEntry`], skipping folders explicitly.
    fn into_change(self) -> Result<Option<ChangeEntry>, PrError> {
        let item = self.item.ok_or_else(|| {
            PrError::Other("Azure returned a file change without an item".to_string())
        })?;
        if item.is_folder {
            return Ok(None);
        }
        let raw_path = item
            .path
            .or(self.original_path)
            .filter(|path| !path.trim_matches('/').is_empty())
            .ok_or_else(|| {
                PrError::Other("Azure returned a file change without a path".to_string())
            })?;
        let path = raw_path.trim_start_matches('/').to_string();
        let new_blob = item.object_id.filter(|id| !id.is_empty());
        let old_blob = item.original_object_id.filter(|id| !id.is_empty());
        let status = self.change_type.file_status();

        match status {
            FileStatus::Added if new_blob.is_none() => {
                return Err(missing_blob_error(&path, "new"));
            }
            FileStatus::Deleted if old_blob.is_none() => {
                return Err(missing_blob_error(&path, "old"));
            }
            FileStatus::Modified if old_blob.is_none() || new_blob.is_none() => {
                let side = if old_blob.is_none() { "old" } else { "new" };
                return Err(missing_blob_error(&path, side));
            }
            _ => {}
        }

        Ok(Some(ChangeEntry {
            path,
            new_blob,
            old_blob,
            status,
        }))
    }
}

fn missing_blob_error(path: &str, side: &str) -> PrError {
    PrError::Other(format!(
        "Azure returned file change {} without {} blob id",
        path, side
    ))
}

fn build_file_diff(
    change: &ChangeEntry,
    old_content: Option<String>,
    new_content: Option<String>,
) -> FileDiff {
    let old_content = old_content.unwrap_or_default();
    let new_content = new_content.unwrap_or_default();
    let is_binary = is_binary_content(&old_content) || is_binary_content(&new_content);
    FileDiff {
        filename: change.path.clone(),
        old_content,
        new_content,
        status: change.status,
        is_binary,
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
    on_http_thread(|| {
        let client = AdoClient::new(org_url, project, repo)?;
        // PR detail is project-scoped (not repo-scoped) in the REST API.
        let url = format!(
            "{}?api-version={}",
            client.git_url(&format!("pullrequests/{}", pr_id)),
            API_VERSION
        );
        let detail: PrDetail = client.get(&url)?;
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
    on_http_thread(|| {
        let client = AdoClient::new(org_url, project, repo)?;
        let url = format!(
            "{}?searchCriteria.status=active&searchCriteria.sourceRefName={}&api-version={}",
            client.git_url(&format!(
                "repositories/{}/pullrequests",
                percent_encode(repo)
            )),
            percent_encode(&format!("refs/heads/{}", branch)),
            API_VERSION
        );
        let list: PrList = client.get(&url)?;
        list.value
            .first()
            .map(|pr| pr.pull_request_id)
            .ok_or_else(|| PrError::NotFound(format!("No active PR found for branch {}", branch)))
    })
}

pub fn load_pr_file_diffs(
    org_url: &str,
    project: &str,
    repo: &str,
    pr_id: u64,
) -> Result<Vec<FileDiff>, PrError> {
    on_http_thread(|| AdoClient::new(org_url, project, repo)?.load_file_diffs(pr_id))
}

/// Keep reqwest's blocking client outside any transitive async runtime.
fn on_http_thread<T, F>(operation: F) -> Result<T, PrError>
where
    T: Send,
    F: FnOnce() -> Result<T, PrError> + Send,
{
    thread::scope(|scope| {
        scope
            .spawn(operation)
            .join()
            .map_err(|_| PrError::Other("Azure HTTP worker panicked".to_string()))?
    })
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
    fn change(v: serde_json::Value) -> Result<Option<ChangeEntry>, PrError> {
        serde_json::from_value::<RawChange>(v)
            .unwrap()
            .into_change()
    }

    #[test]
    fn change_type_parses_add() {
        let add = change(serde_json::json!({
            "changeType": "add",
            "item": { "path": "/src/new.rs", "objectId": "newsha" }
        }))
        .unwrap()
        .unwrap();

        assert_eq!(add.status, FileStatus::Added);
    }

    #[test]
    fn change_type_parses_edit() {
        let edit = change(serde_json::json!({
            "changeType": "edit",
            "item": { "path": "/a.txt", "objectId": "n", "originalObjectId": "o" }
        }))
        .unwrap()
        .unwrap();

        assert_eq!(edit.status, FileStatus::Modified);
    }

    #[test]
    fn change_type_parses_delete() {
        let del = change(serde_json::json!({
            "changeType": "delete",
            "item": { "path": "/gone.rs", "originalObjectId": "o" }
        }))
        .unwrap()
        .unwrap();

        assert_eq!(del.status, FileStatus::Deleted);
    }

    #[test]
    fn change_type_parses_rename_as_modified() {
        let rename = change(serde_json::json!({
            "changeType": "rename",
            "item": {
                "path": "/new.rs",
                "objectId": "n",
                "originalObjectId": "o"
            },
            "originalPath": "/old.rs"
        }))
        .unwrap()
        .unwrap();

        assert_eq!(rename.status, FileStatus::Modified);
    }

    #[test]
    fn unknown_change_type_is_rejected() {
        let result = serde_json::from_value::<RawChange>(serde_json::json!({
            "changeType": "unexpected",
            "item": { "path": "/a.txt", "objectId": "n" }
        }));

        assert!(result.is_err());
    }

    #[test]
    fn skips_folders() {
        assert!(change(serde_json::json!({
            "changeType": "add",
            "item": { "path": "/dir", "isFolder": true }
        }))
        .unwrap()
        .is_none());
    }

    #[test]
    fn falls_back_to_original_path_on_rename() {
        let renamed = change(serde_json::json!({
            "changeType": "rename",
            "item": { "objectId": "n", "originalObjectId": "o" },
            "originalPath": "/old/name.rs"
        }))
        .unwrap()
        .unwrap();
        assert_eq!(renamed.path, "old/name.rs");
    }

    #[test]
    fn malformed_change_without_item_is_rejected() {
        let error = change(serde_json::json!({
            "changeType": "add"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("without an item"));
    }

    #[test]
    fn malformed_change_without_path_is_rejected() {
        let error = change(serde_json::json!({
            "changeType": "add",
            "item": { "objectId": "n" }
        }))
        .unwrap_err();

        assert!(error.to_string().contains("without a path"));
    }

    #[test]
    fn malformed_change_without_required_blob_is_rejected() {
        let error = change(serde_json::json!({
            "changeType": "delete",
            "item": { "path": "/empty.txt" }
        }))
        .unwrap_err();

        assert!(error.to_string().contains("without old blob id"));
    }

    #[test]
    fn empty_added_file_keeps_added_status() {
        let change = change(serde_json::json!({
            "changeType": "add",
            "item": { "path": "/empty.txt", "objectId": "n" }
        }))
        .unwrap()
        .unwrap();
        let diff = build_file_diff(&change, None, Some(String::new()));

        assert_eq!(diff.status, FileStatus::Added);
    }

    #[test]
    fn edit_to_empty_content_keeps_modified_status() {
        let change = change(serde_json::json!({
            "changeType": "edit",
            "item": {
                "path": "/empty.txt",
                "objectId": "n",
                "originalObjectId": "o"
            }
        }))
        .unwrap()
        .unwrap();
        let diff = build_file_diff(&change, Some("content".to_string()), Some(String::new()));

        assert_eq!(diff.status, FileStatus::Modified);
    }

    #[test]
    fn empty_deleted_file_keeps_deleted_status() {
        let change = change(serde_json::json!({
            "changeType": "delete",
            "item": { "path": "/empty.txt", "originalObjectId": "o" }
        }))
        .unwrap()
        .unwrap();
        let diff = build_file_diff(&change, Some(String::new()), None);

        assert_eq!(diff.status, FileStatus::Deleted);
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
