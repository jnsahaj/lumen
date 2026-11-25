use std::fmt;
use std::str::FromStr;

use thiserror::Error;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum CommitReference {
    Single(String),
    Range {
        from: String,
        to: String,
    },
    TripleDots {
        from: String,
        to: String,
    },
    /// Range from a ref to the working tree (`<from>..-`).
    RangeToWorkingTree {
        from: String,
    },
    /// Range from the merge base of a ref and the working-copy parent to the
    /// working tree (`<from>...-`).
    MergeBaseToWorkingTree {
        from: String,
    },
}

impl fmt::Display for CommitReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Single(s) => f.write_str(s),
            Self::Range { from, to } => write!(f, "{from}..{to}"),
            Self::TripleDots { from, to } => write!(f, "{from}...{to}"),
            Self::RangeToWorkingTree { from } => write!(f, "{from}..-"),
            Self::MergeBaseToWorkingTree { from } => write!(f, "{from}...-"),
        }
    }
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

        if let Some((from, to)) = s.split_once("...") {
            let from = if from.is_empty() { "HEAD" } else { from };
            if to == "-" {
                return Ok(Self::MergeBaseToWorkingTree {
                    from: from.to_string(),
                });
            }
            let to = if to.is_empty() { "HEAD" } else { to };
            Ok(Self::TripleDots {
                from: from.to_string(),
                to: to.to_string(),
            })
        } else if let Some((from, to)) = s.split_once("..") {
            let from = if from.is_empty() { "HEAD" } else { from };
            if to == "-" {
                return Ok(Self::RangeToWorkingTree {
                    from: from.to_string(),
                });
            }
            let to = if to.is_empty() { "HEAD" } else { to };
            Ok(Self::Range {
                from: from.to_string(),
                to: to.to_string(),
            })
        } else {
            Ok(Self::Single(s.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reference_parsing_and_display_roundtrip() {
        let cases = [
            ("HEAD", CommitReference::Single("HEAD".to_string()), "HEAD"),
            (
                "main..feature",
                CommitReference::Range {
                    from: "main".to_string(),
                    to: "feature".to_string(),
                },
                "main..feature",
            ),
            (
                "develop..",
                CommitReference::Range {
                    from: "develop".to_string(),
                    to: "HEAD".to_string(),
                },
                "develop..HEAD",
            ),
            (
                "..feature",
                CommitReference::Range {
                    from: "HEAD".to_string(),
                    to: "feature".to_string(),
                },
                "HEAD..feature",
            ),
            (
                "main...feature",
                CommitReference::TripleDots {
                    from: "main".to_string(),
                    to: "feature".to_string(),
                },
                "main...feature",
            ),
            (
                "main..-",
                CommitReference::RangeToWorkingTree {
                    from: "main".to_string(),
                },
                "main..-",
            ),
            (
                "..-",
                CommitReference::RangeToWorkingTree {
                    from: "HEAD".to_string(),
                },
                "HEAD..-",
            ),
            (
                "origin/main...-",
                CommitReference::MergeBaseToWorkingTree {
                    from: "origin/main".to_string(),
                },
                "origin/main...-",
            ),
            (
                "...-",
                CommitReference::MergeBaseToWorkingTree {
                    from: "HEAD".to_string(),
                },
                "HEAD...-",
            ),
            ("@", CommitReference::Single("@".to_string()), "@"),
            ("@-", CommitReference::Single("@-".to_string()), "@-"),
            (
                "@-..@",
                CommitReference::Range {
                    from: "@-".to_string(),
                    to: "@".to_string(),
                },
                "@-..@",
            ),
        ];

        for (input, expected, displayed) in cases {
            let parsed: CommitReference = input.parse().expect("valid reference");
            assert_eq!(parsed, expected, "input: {input}");
            assert_eq!(parsed.to_string(), displayed, "display: {input}");
        }

        assert!(matches!(
            "".parse::<CommitReference>(),
            Err(ReferenceParseError::Empty)
        ));
    }
}
