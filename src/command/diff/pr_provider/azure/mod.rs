//! Azure DevOps provider routing and URL parsing.

mod client;

use crate::command::diff::git::percent_encode;
use crate::command::diff::types::FileDiff;
use crate::command::diff::PrInfo;

use super::{decoded_path_segments, parse_http_url, HttpUrl, PrError, PrProvider};

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

fn strip_http_userinfo(input: &str) -> std::borrow::Cow<'_, str> {
    let Some((scheme, rest)) = input.split_once("://") else {
        return input.into();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let Some((_, host)) = authority.rsplit_once('@') else {
        return input.into();
    };
    format!("{}://{}{}", scheme, host, &rest[authority_end..]).into()
}

pub(super) fn fetch_pr_info(reference: &AzurePrReference) -> Result<PrInfo, PrError> {
    let az = &reference.repository;
    let id = reference.id;
    let meta = client::fetch_pr_metadata(&az.org_url, &az.project, &az.repo, id)?;

    Ok(PrInfo {
        provider: PrProvider::Azure {
            org_url: az.org_url.clone(),
            project: az.project.clone(),
        },
        number: id,
        repo_owner: az.org.clone(),
        repo_name: if meta.repo_name.is_empty() {
            az.repo.clone()
        } else {
            meta.repo_name
        },
        base_ref: strip_ref_prefix(&meta.target_ref),
        head_ref: strip_ref_prefix(&meta.source_ref),
        base_repo_owner: az.org.clone(),
        head_repo_owner: Some(az.org.clone()),
    })
}

pub(super) fn detect_current_branch_pr(
    repository: &AzureRepository,
    branch: &str,
) -> Result<String, PrError> {
    let id = client::detect_active_pr(
        &repository.org_url,
        &repository.project,
        &repository.repo,
        branch,
    )?;
    Ok(id.to_string())
}

pub(super) fn load_pr_file_diffs(
    org_url: &str,
    project: &str,
    pr: &PrInfo,
) -> Result<Vec<FileDiff>, PrError> {
    client::load_pr_file_diffs(org_url, project, &pr.repo_name, pr.number)
}

pub(super) fn file_web_url(org_url: &str, project: &str, pr: &PrInfo, filename: &str) -> String {
    format!(
        "{}/{}/_git/{}/pullrequest/{}?path={}",
        org_url,
        project,
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
