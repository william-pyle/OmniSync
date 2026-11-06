use crate::model::{Action, Rule, RuleKind};
use anyhow::{bail, Context, Result};
use globset::{GlobBuilder, GlobMatcher};

pub fn relative(value: &str) -> Result<String> {
    let value = value.replace('\\', "/");
    if value.is_empty()
        || value.starts_with('/')
        || value.contains(':')
        || value.contains('\0')
        || value
            .split('/')
            .any(|s| s == ".." || s == "." || s.is_empty())
    {
        bail!("Use a root-relative path without empty segments, drive letters or traversal");
    }
    Ok(value)
}
pub struct Matcher {
    rules: Vec<(Rule, Option<GlobMatcher>)>,
}
#[derive(Debug, Clone)]
pub struct Decision {
    pub included: bool,
    pub winning_rule: Option<String>,
    pub reason: String,
}
impl Matcher {
    pub fn new(rules: &[Rule]) -> Result<Self> {
        let mut entries = Vec::new();
        for rule in rules {
            let value = relative(&rule.value)?;
            let matcher = match rule.kind {
                RuleKind::FilenameGlob | RuleKind::PathGlob => {
                    if rule.kind == RuleKind::FilenameGlob && value.contains('/') {
                        bail!("Filename patterns cannot contain a directory separator");
                    }
                    Some(
                        GlobBuilder::new(&value)
                            .case_insensitive(true)
                            .literal_separator(true)
                            .backslash_escape(false)
                            .build()
                            .context("Invalid wildcard pattern")?
                            .compile_matcher(),
                    )
                }
                _ => None,
            };
            entries.push((
                Rule {
                    value,
                    ..rule.clone()
                },
                matcher,
            ));
        }
        Ok(Self { rules: entries })
    }
    pub fn decide(&self, root: &str, path: &str) -> Result<Decision> {
        let path = relative(path)?;
        let lower = path.to_lowercase();
        let mut winner: Option<((u8, usize, u8, u64, usize), &Rule)> = None;
        for (index, (rule, glob)) in self.rules.iter().enumerate() {
            if rule.scope != "global" && rule.scope != root {
                continue;
            }
            let value = rule.value.to_lowercase();
            let (matched, rank, depth) = match rule.kind {
                RuleKind::ExactFile => (lower == value, 4, 0),
                RuleKind::Subtree => (
                    lower == value || lower.starts_with(&(value.clone() + "/")),
                    3,
                    value.split('/').count(),
                ),
                RuleKind::FilenameGlob => (
                    glob.as_ref()
                        .unwrap()
                        .is_match(path.rsplit('/').next().unwrap()),
                    if rule.scope == "global" { 1 } else { 2 },
                    0,
                ),
                RuleKind::PathGlob => (
                    glob.as_ref().unwrap().is_match(&path),
                    if rule.scope == "global" { 1 } else { 2 },
                    0,
                ),
            };
            let key = (rank, depth, u8::from(rule.scope == root), rule.order, index);
            if matched && winner.as_ref().is_none_or(|(k, _)| key > *k) {
                winner = Some((key, rule));
            }
        }
        Ok(match winner {
            Some((_, r)) => Decision {
                included: r.action == Action::Include,
                winning_rule: Some(r.id.clone()),
                reason: format!(
                    "{} by {} ({})",
                    if r.action == Action::Include {
                        "Included"
                    } else {
                        "Excluded"
                    },
                    r.value,
                    r.scope
                ),
            },
            None => Decision {
                included: true,
                winning_rule: None,
                reason: "Included in selected folder".into(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approved_fixtures() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/config.example.json")).unwrap();
        let rules: Vec<Rule> = serde_json::from_value(config["rules"].clone()).unwrap();
        let matcher = Matcher::new(&rules).unwrap();
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/rule-fixtures.json")).unwrap();
        for case in fixtures["cases"].as_array().unwrap() {
            let result = matcher.decide("root-documents", case["path"].as_str().unwrap());
            if case["expected"] == "reject" {
                assert!(result.is_err());
                continue;
            }
            let result = result.unwrap();
            assert_eq!(result.included, case["expected"] == "include", "{case}");
            assert_eq!(
                serde_json::to_value(result.winning_rule).unwrap(),
                case["winningRule"]
            );
        }
    }
    #[test]
    fn precedence_and_boundaries() {
        let rules = vec![
            Rule {
                id: "parent".into(),
                scope: "root".into(),
                kind: RuleKind::Subtree,
                action: Action::Exclude,
                value: "cache".into(),
                order: 8,
            },
            Rule {
                id: "file".into(),
                scope: "root".into(),
                kind: RuleKind::ExactFile,
                action: Action::Include,
                value: "cache/keep.tmp".into(),
                order: 1,
            },
            Rule {
                id: "glob".into(),
                scope: "root".into(),
                kind: RuleKind::FilenameGlob,
                action: Action::Exclude,
                value: "*.tmp".into(),
                order: 999,
            },
        ];
        let matcher = Matcher::new(&rules).unwrap();
        assert!(matcher.decide("root", "cache/keep.tmp").unwrap().included);
        assert!(matcher.decide("root", "cache-other/a").unwrap().included);
        assert!(!matcher.decide("root", "cache/a").unwrap().included);
        for bad in [
            "../a", "a/../b", "C:/x", "/root", "a//b", "a:stream", "a/./b",
        ] {
            assert!(relative(bad).is_err(), "{bad}");
        }
    }
}
