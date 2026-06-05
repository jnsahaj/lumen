//! Azure DevOps provider: URL/remote parsing and the [`PrProvider`] impl. The
//! REST client lives in [`client`].

mod client;

use std::process::Command;

use crate::command::diff::git::percent_encode;
use crate::command::diff::types::FileDiff;
use crate::command::diff::PrInfo;

use super::{read_origin_url, PrProvider};

pub struct AzureProvider;

/// The coordinates of an Azure DevOps repository / PR, parsed from a URL or a
/// git remote.
struct AzureRef {
    /// Organisation base URL, e.g. `https://dev.azure.com/myorg`.
    org_url: String,
    /// Short organisation name, e.g. `myorg`.
    org: String,
    project: String,
    repo: String,
    /// PR id when parsed from a PR URL.
    id: Option<u64>,
}

impl AzureProvider {
    fn resolve_ref(&self, input: &str, repo_override: Option<&str>) -> Result<AzureRef, String> {
        if let Some(parsed) = parse_azure_url(input) {
            return Ok(parsed);
        }
        // Bare PR number: take the coordinates from --origin (if it's an Azure
        // URL) or from the git `origin` remote.
        let id = input
            .parse::<u64>()
            .map_err(|_| format!("Invalid Azure DevOps PR reference: {}", input))?;
        let remote = repo_override
            .filter(|o| self.matches_origin(o))
            .map(|s| s.to_string())
            .or_else(read_origin_url)
            .ok_or_else(|| {
                "Could not determine Azure DevOps repository. Run inside the repo or pass a PR URL."
                    .to_string()
            })?;
        let mut parsed = parse_azure_remote(&remote)
            .ok_or_else(|| format!("Could not parse Azure DevOps remote: {}", remote))?;
        parsed.id = Some(id);
        Ok(parsed)
    }
}

impl PrProvider for AzureProvider {
    fn matches_url(&self, input: &str) -> bool {
        let host_ok = input.contains("dev.azure.com") || input.contains(".visualstudio.com");
        host_ok && (input.contains("/pullrequest/") || input.contains("/_git/"))
    }

    fn matches_origin(&self, origin: &str) -> bool {
        origin.contains("dev.azure.com") || origin.contains(".visualstudio.com")
    }

    fn fetch_pr_info(&self, input: &str, repo_override: Option<&str>) -> Result<PrInfo, String> {
        let az = self.resolve_ref(input, repo_override)?;
        let id = az
            .id
            .ok_or_else(|| format!("No PR id found in: {}", input))?;

        let meta = client::fetch_pr_metadata(&az.org_url, &az.project, &az.repo, id)?;

        Ok(PrInfo {
            provider: &AzureProvider,
            number: id,
            node_id: String::new(),
            repo_owner: az.org.clone(),
            // Prefer the repo name the API reports; fall back to the URL's.
            repo_name: if meta.repo_name.is_empty() {
                az.repo
            } else {
                meta.repo_name
            },
            base_ref: strip_ref_prefix(&meta.target_ref),
            head_ref: strip_ref_prefix(&meta.source_ref),
            base_repo_owner: az.org.clone(),
            head_repo_owner: Some(az.org),
            project: Some(az.project),
            org_url: Some(az.org_url),
        })
    }

    fn detect_current_branch_pr(&self, repo_override: Option<&str>) -> Result<String, String> {
        let remote = repo_override
            .filter(|o| self.matches_origin(o))
            .map(|s| s.to_string())
            .or_else(read_origin_url)
            .ok_or_else(|| "Could not determine Azure DevOps repository.".to_string())?;
        let az = parse_azure_remote(&remote)
            .ok_or_else(|| format!("Could not parse Azure DevOps remote: {}", remote))?;

        let branch_out = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .map_err(|e| format!("Failed to run git: {}", e))?;
        let branch = String::from_utf8_lossy(&branch_out.stdout).trim().to_string();
        if branch.is_empty() {
            return Err("Could not determine the current branch".to_string());
        }

        let id = client::detect_active_pr(&az.org_url, &az.project, &az.repo, &branch)?;
        Ok(id.to_string())
    }

    fn load_pr_file_diffs(&self, pr: &PrInfo) -> Result<Vec<FileDiff>, String> {
        client::load_pr_file_diffs(pr)
    }

    fn file_web_url(&self, pr: &PrInfo, filename: &str) -> Option<String> {
        let org_url = pr.org_url.as_ref()?;
        let project = pr.project.as_ref()?;
        Some(format!(
            "{}/{}/_git/{}/pullrequest/{}?path={}",
            org_url,
            project,
            pr.repo_name,
            pr.number,
            percent_encode(&format!("/{}", filename))
        ))
    }
}

/// Strip a `refs/heads/` (or `refs/`) prefix from an Azure ref name.
fn strip_ref_prefix(ref_name: &str) -> String {
    ref_name
        .strip_prefix("refs/heads/")
        .or_else(|| ref_name.strip_prefix("refs/"))
        .unwrap_or(ref_name)
        .to_string()
}

/// Extract `(org_url, org, project, repo)` from the host+path segments of an
/// Azure DevOps HTTPS URL or remote. Shared by URL and remote parsing.
fn azure_coords_from_parts(parts: &[&str]) -> Option<(String, String, String, String)> {
    let host = *parts.first()?;
    let git_idx = parts.iter().position(|&p| p == "_git")?;
    if git_idx == 0 || git_idx + 1 >= parts.len() {
        return None;
    }
    let project = decode_component(parts[git_idx - 1]);
    let repo = decode_component(parts[git_idx + 1]);

    let (org_url, org) = if host == "dev.azure.com" {
        let org = (*parts.get(1)?).to_string();
        (format!("https://dev.azure.com/{}", org), org)
    } else if let Some(org) = host.strip_suffix(".visualstudio.com") {
        (format!("https://{}", host), org.to_string())
    } else {
        return None;
    };

    Some((org_url, org, project, repo))
}

/// Parse an Azure DevOps PR URL into its coordinates.
///
/// Handles `https://dev.azure.com/{org}/{project}/_git/{repo}/pullrequest/{id}`
/// and `https://{org}.visualstudio.com/{project}/_git/{repo}/pullrequest/{id}`.
fn parse_azure_url(input: &str) -> Option<AzureRef> {
    if !input.starts_with("http") {
        return None;
    }
    let no_query = input.split('?').next().unwrap_or(input);
    let no_scheme = no_query
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let parts: Vec<&str> = no_scheme.split('/').collect();
    let (org_url, org, project, repo) = azure_coords_from_parts(&parts)?;

    let id = parts
        .iter()
        .position(|p| p.eq_ignore_ascii_case("pullrequest"))
        .and_then(|i| parts.get(i + 1))
        .and_then(|s| s.parse::<u64>().ok());

    Some(AzureRef {
        org_url,
        org,
        project,
        repo,
        id,
    })
}

/// Parse an Azure DevOps git remote URL into repository coordinates.
///
/// Handles HTTPS (`https://[org@]dev.azure.com/{org}/{project}/_git/{repo}`,
/// `https://{org}.visualstudio.com/[collection/]{project}/_git/{repo}`) and SSH
/// (`git@ssh.dev.azure.com:v3/{org}/{project}/{repo}`).
fn parse_azure_remote(remote: &str) -> Option<AzureRef> {
    let remote = remote.trim().trim_end_matches(".git");

    // SSH: git@ssh.dev.azure.com:v3/org/project/repo
    if let Some(rest) = remote.split("ssh.dev.azure.com:").nth(1) {
        let mut segs = rest.trim_start_matches('/').split('/');
        // Drop a leading "v3" path component when present.
        let first = segs.next()?;
        let org = if first == "v3" { segs.next()? } else { first };
        let project = segs.next()?;
        let repo = segs.next()?;
        return Some(AzureRef {
            org_url: format!("https://dev.azure.com/{}", org),
            org: org.to_string(),
            project: decode_component(project),
            repo: decode_component(repo),
            id: None,
        });
    }

    // HTTPS variants share the `/_git/` marker.
    let no_scheme = remote
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    // Strip any `user@` userinfo from the host segment.
    let no_userinfo = match no_scheme.split_once('@') {
        Some((_, after)) if after.contains('/') => after,
        _ => no_scheme,
    };
    let parts: Vec<&str> = no_userinfo.split('/').collect();
    let (org_url, org, project, repo) = azure_coords_from_parts(&parts)?;

    Some(AzureRef {
        org_url,
        org,
        project,
        repo,
        id: None,
    })
}

/// Decode the small set of percent-escapes that show up in Azure path segments
/// (notably `%20` for spaces in project names).
fn decode_component(segment: &str) -> String {
    segment.replace("%20", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn azure_matches_pr_urls() {
        assert!(AzureProvider.matches_url("https://dev.azure.com/o/p/_git/r/pullrequest/42"));
        assert!(AzureProvider.matches_url("https://myorg.visualstudio.com/p/_git/r/pullrequest/7"));
        assert!(!AzureProvider.matches_url("https://github.com/owner/repo/pull/123"));
    }

    #[test]
    fn parse_azure_devazure_url() {
        let r = parse_azure_url("https://dev.azure.com/myorg/MyProject/_git/myrepo/pullrequest/55")
            .expect("should parse");
        assert_eq!(r.org_url, "https://dev.azure.com/myorg");
        assert_eq!(r.project, "MyProject");
        assert_eq!(r.repo, "myrepo");
        assert_eq!(r.id, Some(55));
    }

    #[test]
    fn parse_azure_visualstudio_url() {
        let r = parse_azure_url("https://myorg.visualstudio.com/MyProject/_git/myrepo/pullrequest/9")
            .expect("should parse");
        assert_eq!(r.org_url, "https://myorg.visualstudio.com");
        assert_eq!(r.project, "MyProject");
        assert_eq!(r.repo, "myrepo");
        assert_eq!(r.id, Some(9));
    }

    #[test]
    fn parse_azure_url_with_encoded_project() {
        let r = parse_azure_url("https://dev.azure.com/org/My%20Project/_git/repo/pullrequest/1")
            .expect("should parse");
        assert_eq!(r.project, "My Project");
    }

    #[test]
    fn parse_azure_https_remote() {
        let r = parse_azure_remote("https://myorg@dev.azure.com/myorg/MyProject/_git/myrepo")
            .expect("should parse");
        assert_eq!(r.org_url, "https://dev.azure.com/myorg");
        assert_eq!(r.project, "MyProject");
        assert_eq!(r.repo, "myrepo");
    }

    #[test]
    fn parse_azure_ssh_remote() {
        let r = parse_azure_remote("git@ssh.dev.azure.com:v3/myorg/MyProject/myrepo")
            .expect("should parse");
        assert_eq!(r.org_url, "https://dev.azure.com/myorg");
        assert_eq!(r.project, "MyProject");
        assert_eq!(r.repo, "myrepo");
    }

    #[test]
    fn strip_ref_prefixes() {
        assert_eq!(strip_ref_prefix("refs/heads/main"), "main");
        assert_eq!(strip_ref_prefix("refs/tags/v1"), "tags/v1");
        assert_eq!(strip_ref_prefix("feature/x"), "feature/x");
    }

    #[test]
    fn encodes_path_query() {
        assert_eq!(percent_encode("/src/main.rs"), "%2Fsrc%2Fmain.rs");
    }
}
