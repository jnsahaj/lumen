//! Pull-request hosting provider routing.

mod azure;
mod github;
mod viewed;

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

use super::types::FileDiff;
use crate::vcs::VcsBackend;

use azure::{AzurePrReference, AzureRepository};
use github::{GitHubPrReference, GitHubRepository};
pub(crate) use viewed::ViewedFileSync;

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

#[derive(Clone)]
pub enum PrInfo {
    GitHub(github::GitHubPr),
    Azure(azure::AzurePr),
}

impl PrInfo {
    pub fn number(&self) -> u64 {
        match self {
            Self::GitHub(pr) => pr.number,
            Self::Azure(pr) => pr.number,
        }
    }

    pub fn base_ref(&self) -> &str {
        match self {
            Self::GitHub(pr) => &pr.base_ref,
            Self::Azure(pr) => &pr.base_ref,
        }
    }

    pub fn head_ref(&self) -> &str {
        match self {
            Self::GitHub(pr) => &pr.head_ref,
            Self::Azure(pr) => &pr.head_ref,
        }
    }

    pub fn base_repo_owner(&self) -> &str {
        match self {
            Self::GitHub(pr) => &pr.base_repo_owner,
            Self::Azure(pr) => &pr.org,
        }
    }

    pub fn head_repo_owner(&self) -> Option<&str> {
        match self {
            Self::GitHub(pr) => pr.head_repo_owner.as_deref(),
            Self::Azure(pr) => Some(&pr.org),
        }
    }

    pub fn load_file_diffs(&self) -> Result<Vec<FileDiff>, PrError> {
        match self {
            Self::GitHub(pr) => github::load_pr_file_diffs(pr),
            Self::Azure(pr) => azure::load_pr_file_diffs(pr),
        }
    }

    fn viewed_file_provider(&self) -> Option<ViewedFileProvider> {
        match self {
            Self::GitHub(pr) => Some(ViewedFileProvider { pr: pr.clone() }),
            Self::Azure(_) => None,
        }
    }

    pub fn file_web_url(&self, filename: &str) -> String {
        match self {
            Self::GitHub(pr) => github::file_web_url(pr, filename),
            Self::Azure(pr) => azure::file_web_url(pr, filename),
        }
    }
}

#[derive(Clone)]
struct ViewedFileProvider {
    pr: github::GitHubPr,
}

impl ViewedFileProvider {
    fn fetch(&self) -> Result<HashSet<String>, PrError> {
        github::fetch_viewed_files(&self.pr)
    }

    fn set(&self, path: &str, viewed: bool) -> Result<(), PrError> {
        github::set_file_viewed(&self.pr.node_id, path, viewed)
    }
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
    origin_error: Option<String>,
}

impl RepositoryContext {
    pub fn resolve(backend: Option<&dyn VcsBackend>, repository_override: Option<&str>) -> Self {
        let (origin, origin_error) = match repository_override {
            Some(origin) => (Some(origin.to_owned()), None),
            None => match backend.map(VcsBackend::origin_url).transpose() {
                Ok(origin) => (origin.flatten(), None),
                Err(error) => (None, Some(error.to_string())),
            },
        };
        let repository = origin.as_deref().and_then(parse_repository);

        Self {
            origin,
            repository,
            origin_error,
        }
    }

    #[cfg(test)]
    fn from_sources(repository_override: Option<&str>, backend_origin: Option<&str>) -> Self {
        let origin = repository_override
            .map(str::to_owned)
            .or_else(|| backend_origin.map(str::to_owned));
        let repository = origin.as_deref().and_then(parse_repository);
        Self {
            origin,
            repository,
            origin_error: None,
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
            None => Err(PrError::InvalidRef(repository_context_error(context))),
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
        PrReference::GitHub(reference) => github::fetch_pr_info(&reference).map(PrInfo::GitHub),
        PrReference::Azure(reference) => azure::fetch_pr_info(&reference).map(PrInfo::Azure),
        PrReference::Number(_) => {
            unreachable!("bare PR numbers are resolved using repository context")
        }
    }
}

pub fn detect_current_branch_pr(
    context: &RepositoryContext,
    backend: Option<&dyn VcsBackend>,
) -> Result<PrInfo, PrError> {
    let backend = backend.ok_or_else(|| {
        PrError::NotFound("could not determine the current branch or bookmark".to_string())
    })?;
    let branch = backend
        .get_pr_source_branch()
        .map_err(|error| PrError::Other(error.to_string()))?
        .ok_or_else(|| {
            PrError::NotFound("could not determine the current branch or bookmark".to_string())
        })?;
    match &context.repository {
        Some(Repository::GitHub(repo)) => {
            let reference = github::detect_current_branch_pr(repo, &branch)?;
            github::fetch_pr_info(&reference).map(PrInfo::GitHub)
        }
        Some(Repository::Azure(repo)) => {
            azure::detect_current_branch_pr(repo, &branch).map(PrInfo::Azure)
        }
        None => Err(PrError::InvalidRef(repository_context_error(context))),
    }
}

fn repository_context_error(context: &RepositoryContext) -> String {
    match (&context.origin, &context.origin_error) {
        (Some(origin), _) => format!("unsupported repository origin: {}", safe_diagnostic(origin)),
        (None, Some(error)) => format!("could not read repository origin: {error}"),
        (None, None) => {
            "could not determine repository; configure origin or pass --origin".to_string()
        }
    }
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

pub(super) fn percent_encode(segment: &str) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
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
        );

        assert!(matches!(context.repository, Some(Repository::Azure(_))));
    }

    #[test]
    fn github_repository_origin_accepts_https_credentials() {
        let context = RepositoryContext::from_sources(
            None,
            Some("https://user:TOKEN@github.com/owner/repo.git"),
        );

        assert!(matches!(context.repository, Some(Repository::GitHub(_))));
    }

    #[test]
    fn unsupported_origin_error_redacts_https_credentials() {
        let context = RepositoryContext::from_sources(
            None,
            Some("https://user:SECRET@example.com/owner/repo.git"),
        );

        let error = resolve_pr_reference("12", &context).expect_err("unsupported origin");

        assert_eq!(
            error.to_string(),
            "invalid PR reference: unsupported repository origin: https://example.com/owner/repo.git"
        );
    }

    #[test]
    fn invalid_reference_error_redacts_https_credentials() {
        let context = RepositoryContext::from_sources(None, None);

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
