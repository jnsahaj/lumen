//! Pull-request hosting provider routing.

mod azure;
mod github;

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::thread;

use super::types::FileDiff;
use super::PrInfo;
use crate::vcs::VcsBackend;

use azure::{AzurePrReference, AzureRepository};
use github::{GitHubPrReference, GitHubRepository};

#[derive(Debug, thiserror::Error)]
pub enum PrError {
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid PR reference: {0}")]
    InvalidRef(String),
    #[error("{0}")]
    Other(String),
}

impl From<String> for PrError {
    fn from(s: String) -> Self {
        PrError::Other(s)
    }
}

impl From<&str> for PrError {
    fn from(s: &str) -> Self {
        PrError::Other(s.to_string())
    }
}

/// Provider-specific coordinates for a resolved pull request.
#[derive(Clone, Debug)]
pub enum PrProvider {
    GitHub { node_id: String },
    Azure { org_url: String, project: String },
}

#[derive(Clone, Debug)]
enum Repository {
    GitHub(GitHubRepository),
    Azure(AzureRepository),
}

/// Repository information read once from the selected VCS backend.
#[derive(Clone, Debug)]
pub struct RepositoryContext {
    origin: Option<String>,
    repository: Option<Repository>,
    current_branch: Option<String>,
}

impl RepositoryContext {
    pub fn resolve(backend: Option<&dyn VcsBackend>, repository_override: Option<&str>) -> Self {
        let origin = repository_override
            .map(str::to_owned)
            .or_else(|| backend.and_then(|backend| backend.origin_url().ok().flatten()));
        let repository = origin.as_deref().and_then(parse_repository);
        let current_branch =
            backend.and_then(|backend| backend.get_current_branch().ok().flatten());

        Self {
            origin,
            repository,
            current_branch,
        }
    }

    #[cfg(test)]
    fn from_sources(
        repository_override: Option<&str>,
        backend_origin: Option<&str>,
        current_branch: Option<&str>,
    ) -> Self {
        let origin = repository_override
            .map(str::to_owned)
            .or_else(|| backend_origin.map(str::to_owned));
        let repository = origin.as_deref().and_then(parse_repository);
        Self {
            origin,
            repository,
            current_branch: current_branch.map(str::to_owned),
        }
    }
}

#[derive(Clone, Debug)]
enum PrReference {
    GitHub(GitHubPrReference),
    Azure(AzurePrReference),
    Number(u64),
}

fn parse_pr_reference(input: &str) -> Option<PrReference> {
    if let Ok(number) = input.parse::<u64>() {
        return Some(PrReference::Number(number));
    }
    let url = parse_http_url(input)?;
    github::parse_pr_url(url)
        .map(PrReference::GitHub)
        .or_else(|| azure::parse_pr_url(url).map(PrReference::Azure))
}

fn parse_repository(input: &str) -> Option<Repository> {
    azure::parse_repository(input)
        .map(Repository::Azure)
        .or_else(|| github::parse_repository(input).map(Repository::GitHub))
}

fn resolve_pr_reference(input: &str, context: &RepositoryContext) -> Result<PrReference, PrError> {
    match parse_pr_reference(input) {
        Some(PrReference::Number(number)) => match &context.repository {
            Some(Repository::GitHub(repo)) => Ok(PrReference::GitHub(repo.with_number(number))),
            Some(Repository::Azure(repo)) => Ok(PrReference::Azure(repo.with_number(number))),
            None => Err(PrError::InvalidRef(match &context.origin {
                Some(origin) => {
                    format!("unsupported repository origin: {}", safe_diagnostic(origin))
                }
                None => {
                    "could not determine repository; configure origin or pass --origin".to_string()
                }
            })),
        },
        Some(reference) => Ok(reference),
        None => Err(PrError::InvalidRef(format!(
            "{}. Use a PR number or an exact GitHub/Azure DevOps PR URL.",
            safe_diagnostic(input)
        ))),
    }
}

pub fn is_pr_reference(input: &str) -> bool {
    parse_pr_reference(input).is_some()
}

pub fn fetch_pr_info(input: &str, context: &RepositoryContext) -> Result<PrInfo, PrError> {
    match resolve_pr_reference(input, context)? {
        PrReference::GitHub(reference) => github::fetch_pr_info(&reference),
        PrReference::Azure(reference) => azure::fetch_pr_info(&reference),
        PrReference::Number(_) => {
            unreachable!("bare PR numbers are resolved using repository context")
        }
    }
}

pub fn detect_current_branch_pr(context: &RepositoryContext) -> Result<String, PrError> {
    let branch = context.current_branch.as_deref().ok_or_else(|| {
        PrError::NotFound("could not determine the current branch or bookmark".to_string())
    })?;
    match &context.repository {
        Some(Repository::GitHub(repo)) => github::detect_current_branch_pr(repo, branch),
        Some(Repository::Azure(repo)) => azure::detect_current_branch_pr(repo, branch),
        None => Err(PrError::InvalidRef(match &context.origin {
            Some(origin) => {
                format!("unsupported repository origin: {}", safe_diagnostic(origin))
            }
            None => "could not determine repository; configure origin or pass --origin".to_string(),
        })),
    }
}

pub fn load_pr_file_diffs(pr: &PrInfo) -> Result<Vec<FileDiff>, PrError> {
    match &pr.provider {
        PrProvider::GitHub { .. } => github::load_pr_file_diffs(pr),
        PrProvider::Azure { org_url, project } => azure::load_pr_file_diffs(org_url, project, pr),
    }
}

/// `None` means the provider does not support per-file viewed state.
pub fn fetch_viewed_files(pr: &PrInfo) -> Result<Option<HashSet<String>>, PrError> {
    match &pr.provider {
        PrProvider::GitHub { .. } => github::fetch_viewed_files(pr).map(Some),
        PrProvider::Azure { .. } => Ok(None),
    }
}

pub fn supports_viewed_files(pr: &PrInfo) -> bool {
    matches!(&pr.provider, PrProvider::GitHub { .. })
}

pub fn pr_file_web_url(pr: &PrInfo, filename: &str) -> Option<String> {
    match &pr.provider {
        PrProvider::GitHub { .. } => Some(github::file_web_url(pr, filename)),
        PrProvider::Azure { org_url, project } => {
            Some(azure::file_web_url(org_url, project, pr, filename))
        }
    }
}

pub fn mark_file_as_viewed_async(pr: &PrInfo, file_path: &str) {
    set_file_viewed_async(pr, file_path, true);
}

pub fn unmark_file_as_viewed_async(pr: &PrInfo, file_path: &str) {
    set_file_viewed_async(pr, file_path, false);
}

fn set_file_viewed_async(pr: &PrInfo, file_path: &str, viewed: bool) {
    if !matches!(&pr.provider, PrProvider::GitHub { .. }) {
        return;
    }
    let pr = pr.clone();
    let path = file_path.to_string();
    thread::spawn(move || {
        let PrProvider::GitHub { node_id } = &pr.provider else {
            return;
        };
        let _ = github::set_file_viewed(node_id, &path, viewed);
    });
}

#[derive(Clone, Copy)]
pub(super) struct HttpUrl<'a> {
    pub host: &'a str,
    pub path: &'a str,
}

pub(super) fn strip_http_userinfo(input: &str) -> Cow<'_, str> {
    let Some((scheme, rest)) = input.split_once("://") else {
        return input.into();
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return input.into();
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let Some((_, host)) = authority.rsplit_once('@') else {
        return input.into();
    };
    format!("{}://{}{}", scheme, host, &rest[authority_end..]).into()
}

struct SafeDiagnostic<'a>(&'a str);

impl fmt::Display for SafeDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(strip_http_userinfo(self.0).as_ref())
    }
}

fn safe_diagnostic(input: &str) -> SafeDiagnostic<'_> {
    SafeDiagnostic(input)
}

pub(super) fn parse_http_url(input: &str) -> Option<HttpUrl<'_>> {
    let (scheme, rest) = input.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() || authority.contains('@') || authority.contains(':') {
        return None;
    }
    let suffix = &rest[authority_end..];
    let path_end = suffix.find(['?', '#']).unwrap_or(suffix.len());
    let path = &suffix[..path_end];
    Some(HttpUrl {
        host: authority,
        path,
    })
}

pub(super) fn decoded_path_segments(path: &str) -> Option<Vec<String>> {
    let path = path.strip_prefix('/')?;
    let path = path.strip_suffix('/').unwrap_or(path);
    if path.is_empty() {
        return Some(Vec::new());
    }
    path.split('/').map(decode_path_segment).collect()
}

fn decode_path_segment(segment: &str) -> Option<String> {
    if segment.is_empty() {
        return None;
    }
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_only_exact_provider_urls() {
        assert!(is_pr_reference("123"));
        assert!(is_pr_reference("https://github.com/o/r/pull/1"));
        assert!(is_pr_reference(
            "https://dev.azure.com/o/p/_git/r/pullrequest/1"
        ));
        assert!(!is_pr_reference("https://github.com.evil.test/o/r/pull/1"));
        assert!(!is_pr_reference(
            "https://evil.test/dev.azure.com/o/p/_git/r/pullrequest/1"
        ));
        assert!(!is_pr_reference("https://github.com/o/r/issues/1"));
    }

    #[test]
    fn repository_override_replaces_backend_origin() {
        let context = RepositoryContext::from_sources(
            Some("https://dev.azure.com/org/project/_git/repo"),
            Some("git@github.com:owner/repo.git"),
            Some("feature"),
        );

        assert!(matches!(context.repository, Some(Repository::Azure(_))));
        assert_eq!(context.current_branch.as_deref(), Some("feature"));
    }

    #[test]
    fn github_repository_origin_accepts_https_credentials() {
        let context = RepositoryContext::from_sources(
            None,
            Some("https://user:TOKEN@github.com/owner/repo.git"),
            Some("feature"),
        );

        assert!(matches!(context.repository, Some(Repository::GitHub(_))));
    }

    #[test]
    fn unsupported_origin_error_redacts_https_credentials() {
        let context = RepositoryContext::from_sources(
            None,
            Some("https://user:SECRET@example.com/owner/repo.git"),
            Some("feature"),
        );

        let error = resolve_pr_reference("12", &context).expect_err("unsupported origin");

        assert_eq!(
            error.to_string(),
            "invalid PR reference: unsupported repository origin: https://example.com/owner/repo.git"
        );
    }

    #[test]
    fn invalid_reference_error_redacts_https_credentials() {
        let context = RepositoryContext::from_sources(None, None, None);

        let error = resolve_pr_reference(
            "https://user:SECRET@example.com/owner/repo/pull/1",
            &context,
        )
        .expect_err("unknown provider");

        assert!(!error.to_string().contains("SECRET"));
    }

    #[test]
    fn decodes_all_percent_encoded_utf8_bytes() {
        assert_eq!(
            decode_path_segment("My%2BProject").as_deref(),
            Some("My+Project")
        );
        assert_eq!(decode_path_segment("caf%C3%A9").as_deref(), Some("café"));
        assert!(decode_path_segment("bad%2").is_none());
    }
}
