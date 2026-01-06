use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum CommitReference {
    Single(String),
    Range { from: String, to: String },
    TripleDots { from: String, to: String },
}

#[derive(Debug, Error)]
pub enum ReferenceParseError {
    #[error("empty reference string")]
    Empty,
}

impl FromStr for CommitReference {
    type Err = ReferenceParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ReferenceParseError::Empty);
        }

        // Handle the ... and .. cases
        if let Some((from, to)) = s.split_once("...") {
            let from = if from.is_empty() { "HEAD" } else { from };
            let to = if to.is_empty() { "HEAD" } else { to };

            Ok(CommitReference::TripleDots {
                from: from.to_string(),
                to: to.to_string(),
            })
        } else if let Some((from, to)) = s.split_once("..") {
            let from = if from.is_empty() { "HEAD" } else { from };
            let to = if to.is_empty() { "HEAD" } else { to };

            Ok(CommitReference::Range {
                from: from.to_string(),
                to: to.to_string(),
            })
        } else {
            Ok(CommitReference::Single(s.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct TestCli {
        reference: CommitReference,
    }

    #[test]
    fn test_single_commit() {
        assert_eq!(
            "HEAD".parse::<CommitReference>().unwrap(),
            CommitReference::Single("HEAD".to_string())
        );
    }

    #[test]
    fn test_full_range() {
        assert_eq!(
            "main..feature".parse::<CommitReference>().unwrap(),
            CommitReference::Range {
                from: "main".to_string(),
                to: "feature".to_string(),
            }
        );
    }

    #[test]
    fn test_from_only_range() {
        assert_eq!(
            "develop..".parse::<CommitReference>().unwrap(),
            CommitReference::Range {
                from: "develop".to_string(),
                to: "HEAD".to_string(),
            }
        );
    }

    #[test]
    fn test_to_only_range() {
        assert_eq!(
            "..feature".parse::<CommitReference>().unwrap(),
            CommitReference::Range {
                from: "HEAD".to_string(),
                to: "feature".to_string(),
            }
        );
    }

    #[test]
    fn test_clap_integration() {
        // Test full range
        let cli = TestCli::try_parse_from(&["test", "main..feature"]).unwrap();
        assert!(matches!(
            cli.reference,
            CommitReference::Range { from, to }
            if from == "main" && to == "feature"
        ));

        // Test from-only range
        let cli = TestCli::try_parse_from(&["test", "develop.."]).unwrap();
        assert!(matches!(
            cli.reference,
            CommitReference::Range { from, to }
            if from == "develop" && to == "HEAD"
        ));

        // Test to-only range
        let cli = TestCli::try_parse_from(&["test", "..feature"]).unwrap();
        assert!(matches!(
            cli.reference,
            CommitReference::Range { from, to }
            if from == "HEAD" && to == "feature"
        ));
    }

    #[test]
    fn test_empty_reference() {
        assert!(matches!(
            "".parse::<CommitReference>(),
            Err(ReferenceParseError::Empty)
        ));
    }

    // jj-style ref syntax tests
    #[test]
    fn test_jj_working_copy_ref() {
        assert_eq!(
            "@".parse::<CommitReference>().unwrap(),
            CommitReference::Single("@".to_string())
        );
    }

    #[test]
    fn test_jj_parent_ref() {
        assert_eq!(
            "@-".parse::<CommitReference>().unwrap(),
            CommitReference::Single("@-".to_string())
        );
    }

    #[test]
    fn test_jj_grandparent_ref() {
        assert_eq!(
            "@--".parse::<CommitReference>().unwrap(),
            CommitReference::Single("@--".to_string())
        );
    }

    #[test]
    fn test_jj_change_id_prefix() {
        // jj change IDs are short alphanumeric prefixes
        assert_eq!(
            "xyz".parse::<CommitReference>().unwrap(),
            CommitReference::Single("xyz".to_string())
        );
        assert_eq!(
            "ksrm".parse::<CommitReference>().unwrap(),
            CommitReference::Single("ksrm".to_string())
        );
    }

    #[test]
    fn test_jj_range_syntax() {
        // jj supports @ in ranges
        assert_eq!(
            "@-..@".parse::<CommitReference>().unwrap(),
            CommitReference::Range {
                from: "@-".to_string(),
                to: "@".to_string(),
            }
        );
    }
}
