//! Azure DevOps provider routing and URL parsing.

mod client;

use crate::command::diff::types::FileDiff;

use super::{
    decoded_path_segments, parse_http_url, percent_encode, strip_http_userinfo, HttpUrl, PrError,
};

#[derive(Clone, Debug)]
pub(super) struct AzureRepository {
    org_url: String,
    org: String,
    project: String,
    repo: String,
}

impl AzureRepository {
    pub(super) fn with_number(&self, id: u64) -> AzurePrReference {
        AzurePrReference {
            repository: self.clone(),
            id,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct AzurePrReference {
    repository: AzureRepository,
    id: u64,
}

#[derive(Clone)]
pub(crate) struct AzurePr {
    client: client::AdoClient,
    pub(super) number: u64,
    pub(super) org: String,
    pub(super) repo_name: String,
    pub(super) base_ref: String,
    pub(super) head_ref: String,
    org_url: String,
    project: String,
}

impl AzurePr {
    fn resolved(
        repository: &AzureRepository,
        client: client::AdoClient,
        number: u64,
        meta: client::AzurePrMeta,
    ) -> Self {
        Self {
            client,
            number,
            org: repository.org.clone(),
            repo_name: if meta.repo_name.is_empty() {
                repository.repo.clone()
            } else {
                meta.repo_name
            },
            base_ref: strip_ref_prefix(&meta.target_ref),
            head_ref: strip_ref_prefix(&meta.source_ref),
            org_url: repository.org_url.clone(),
            project: repository.project.clone(),
        }
    }
}

pub(super) fn parse_pr_url(url: HttpUrl<'_>) -> Option<AzurePrReference> {
    let parts = decoded_path_segments(url.path)?;
    let (repository, pullrequest_index) = if url.host.eq_ignore_ascii_case("dev.azure.com") {
        if parts.len() != 6 || parts[2] != "_git" {
            return None;
        }
        (
            AzureRepository {
                org_url: format!("https://dev.azure.com/{}", parts[0]),
                org: parts[0].clone(),
                project: parts[1].clone(),
                repo: parts[3].clone(),
            },
            4,
        )
    } else {
        let org = visualstudio_org(url.host)?;
        let (project_index, git_index, pr_index) = match parts.len() {
            5 => (0, 1, 3),
            6 => (1, 2, 4),
            _ => return None,
        };
        if parts[git_index] != "_git" {
            return None;
        }
        (
            AzureRepository {
                org_url: format!("https://{}", url.host),
                org,
                project: parts[project_index].clone(),
                repo: parts[git_index + 1].clone(),
            },
            pr_index,
        )
    };

    if !parts[pullrequest_index].eq_ignore_ascii_case("pullrequest") {
        return None;
    }
    let id = parts.get(pullrequest_index + 1)?.parse().ok()?;
    Some(AzurePrReference { repository, id })
}

pub(super) fn parse_repository(input: &str) -> Option<AzureRepository> {
    let input = input.trim().trim_end_matches('/');
    if let Some(repository) = parse_ssh_repository(input) {
        return Some(repository);
    }

    let normalized = strip_http_userinfo(input);
    let url = parse_http_url(normalized.as_ref())?;
    let mut parts = decoded_path_segments(url.path)?;
    let repo = parts.last_mut()?;
    *repo = repo.trim_end_matches(".git").to_string();
    if repo.is_empty() {
        return None;
    }

    if url.host.eq_ignore_ascii_case("dev.azure.com") {
        if parts.len() != 4 || parts[2] != "_git" {
            return None;
        }
        Some(AzureRepository {
            org_url: format!("https://dev.azure.com/{}", parts[0]),
            org: parts[0].clone(),
            project: parts[1].clone(),
            repo: parts[3].clone(),
        })
    } else {
        let org = visualstudio_org(url.host)?;
        let (project_index, git_index) = match parts.len() {
            3 => (0, 1),
            4 => (1, 2),
            _ => return None,
        };
        if parts[git_index] != "_git" {
            return None;
        }
        Some(AzureRepository {
            org_url: format!("https://{}", url.host),
            org,
            project: parts[project_index].clone(),
            repo: parts[git_index + 1].clone(),
        })
    }
}

fn visualstudio_org(host: &str) -> Option<String> {
    let lowercase = host.to_ascii_lowercase();
    let org = lowercase.strip_suffix(".visualstudio.com")?;
    if org.is_empty() || org.contains('.') {
        return None;
    }
    Some(org.to_string())
}

fn parse_ssh_repository(input: &str) -> Option<AzureRepository> {
    let path = input
        .strip_prefix("git@ssh.dev.azure.com:")
        .or_else(|| input.strip_prefix("ssh://git@ssh.dev.azure.com/"))?;
    let parts = decoded_path_segments(&format!("/{}", path))?;
    let (org_index, project_index, repo_index) = if parts.len() == 4 && parts[0] == "v3" {
        (1, 2, 3)
    } else if parts.len() == 3 {
        (0, 1, 2)
    } else {
        return None;
    };
    let repo = parts[repo_index].trim_end_matches(".git").to_string();
    if repo.is_empty() {
        return None;
    }
    Some(AzureRepository {
        org_url: format!("https://dev.azure.com/{}", parts[org_index]),
        org: parts[org_index].clone(),
        project: parts[project_index].clone(),
        repo,
    })
}

pub(super) fn fetch_pr_info(reference: &AzurePrReference) -> Result<AzurePr, PrError> {
    let az = &reference.repository;
    let id = reference.id;
    let (client, meta) = client::resolve_pr(&az.org_url, &az.project, &az.repo, id)?;

    Ok(AzurePr::resolved(az, client, id, meta))
}

pub(super) fn detect_current_branch_pr(
    repository: &AzureRepository,
    branch: &str,
) -> Result<AzurePr, PrError> {
    let (client, id, meta) = client::detect_active_pr(
        &repository.org_url,
        &repository.project,
        &repository.repo,
        branch,
    )?;
    Ok(AzurePr::resolved(repository, client, id, meta))
}

pub(super) fn load_pr_file_diffs(pr: &AzurePr) -> Result<Vec<FileDiff>, PrError> {
    client::load_pr_file_diffs(&pr.client, pr.number)
}

pub(super) fn file_web_url(pr: &AzurePr, filename: &str) -> String {
    format!(
        "{}/{}/_git/{}/pullrequest/{}?path={}",
        pr.org_url,
        pr.project,
        pr.repo_name,
        pr.number,
        percent_encode(&format!("/{}", filename))
    )
}

fn strip_ref_prefix(ref_name: &str) -> String {
    ref_name
        .strip_prefix("refs/heads/")
        .or_else(|| ref_name.strip_prefix("refs/"))
        .unwrap_or(ref_name)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_azure_pr_urls() {
        let reference =
            parse_http_url("https://dev.azure.com/myorg/MyProject/_git/myrepo/pullrequest/55")
                .and_then(parse_pr_url)
                .expect("should parse");
        assert_eq!(reference.repository.org_url, "https://dev.azure.com/myorg");
        assert_eq!(reference.repository.project, "MyProject");
        assert_eq!(reference.repository.repo, "myrepo");
        assert_eq!(reference.id, 55);

        assert!(parse_http_url(
            "https://dev.azure.com.evil.test/myorg/MyProject/_git/myrepo/pullrequest/55",
        )
        .and_then(parse_pr_url)
        .is_none());
        assert!(parse_http_url(
            "https://dev.azure.com/myorg/MyProject/_git/myrepo/pullrequest/55/files",
        )
        .and_then(parse_pr_url)
        .is_none());
    }

    #[test]
    fn azure_repository_url_is_not_a_pr_reference() {
        let url = "https://dev.azure.com/org/project/_git/repo";
        assert!(parse_http_url(url).and_then(parse_pr_url).is_none());
        assert!(parse_repository(url).is_some());
    }

    #[test]
    fn fully_decodes_azure_path_segments() {
        let reference =
            parse_http_url("https://dev.azure.com/org/My%2BProject/_git/caf%C3%A9/pullrequest/1")
                .and_then(parse_pr_url)
                .expect("should parse");
        assert_eq!(reference.repository.project, "My+Project");
        assert_eq!(reference.repository.repo, "café");
    }

    #[test]
    fn parses_visualstudio_pr_url() {
        let reference =
            parse_http_url("https://myorg.visualstudio.com/MyProject/_git/myrepo/pullrequest/9")
                .and_then(parse_pr_url)
                .expect("should parse");
        assert_eq!(
            reference.repository.org_url,
            "https://myorg.visualstudio.com"
        );
        assert_eq!(reference.repository.project, "MyProject");
        assert_eq!(reference.id, 9);
    }

    #[test]
    fn parses_azure_https_and_ssh_repositories() {
        let https =
            parse_repository("https://myorg@dev.azure.com/myorg/My%20Project/_git/myrepo.git")
                .expect("https");
        let ssh =
            parse_repository("git@ssh.dev.azure.com:v3/myorg/My%20Project/myrepo").expect("ssh");
        assert_eq!(https.project, "My Project");
        assert_eq!(https.repo, "myrepo");
        assert_eq!(ssh.project, "My Project");
        assert_eq!(ssh.repo, "myrepo");
    }

    #[test]
    fn strip_ref_prefixes() {
        assert_eq!(strip_ref_prefix("refs/heads/main"), "main");
        assert_eq!(strip_ref_prefix("refs/tags/v1"), "tags/v1");
        assert_eq!(strip_ref_prefix("feature/x"), "feature/x");
    }
}
