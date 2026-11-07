// SPDX-License-Identifier: BSD-3-Clause
//! Release selection, the fork's change set, and the rules.

use crate::data::{Release, Version};
use crate::git::{Change, Repo};
use crate::parse::{board_assignments, board_of, kconfig_definitions};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_KCONFIG_UNDEFINED: &str = "kconfig-undefined";

pub struct Options {
    /// Release the fork is based on; detected from HEAD's history when not given.
    pub base: Option<String>,
    /// Release the fork will merge.
    pub target: String,
    /// Check every board file in HEAD instead of the files the fork changed.
    pub all_files: bool,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub base: Option<String>,
    pub target: String,
    pub head: String,
    pub scope: &'static str,
    pub files_checked: usize,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub rule: &'static str,
    pub file: String,
    pub line: usize,
    pub symbol: String,
    pub text: String,
    /// Whether the base release defines the symbol; null without a base.
    pub defined_at_base: Option<bool>,
    pub message: String,
}

/// The newest data release whose commit HEAD contains. `main` is never chosen.
pub fn detect_base<'a>(repo: &Repo, releases: &'a BTreeMap<String, Release>) -> Option<&'a str> {
    releases
        .values()
        .filter(|r| r.release != "main")
        .filter(|r| repo.has_commit(&r.source.commit) && repo.is_ancestor_of_head(&r.source.commit))
        .max_by_key(|r| Version::parse(&r.release))
        .map(|r| r.release.as_str())
}

fn known<'a>(releases: &'a BTreeMap<String, Release>, name: &str) -> Result<&'a Release, String> {
    releases.get(name).ok_or_else(|| {
        let names: Vec<&str> = releases.keys().map(String::as_str).collect();
        format!(
            "no data for release {name}; known releases: {}",
            names.join(", ")
        )
    })
}

pub fn run(
    repo: &Repo,
    releases: &BTreeMap<String, Release>,
    options: &Options,
) -> Result<Report, String> {
    let target = known(releases, &options.target)?;
    let base_name = match &options.base {
        Some(name) => Some(name.clone()),
        None => match detect_base(repo, releases) {
            Some(name) => Some(name.to_string()),
            None if options.all_files => None,
            None => {
                return Err(
                    "no release in the data is an ancestor of HEAD; name the base with --base"
                        .into(),
                )
            }
        },
    };
    let base = base_name
        .as_deref()
        .map(|name| known(releases, name))
        .transpose()?;
    if let Some(base) = base {
        if Version::parse(&target.release) <= Version::parse(&base.release) {
            return Err(format!(
                "target {} is not newer than base {}",
                target.release, base.release
            ));
        }
        if !repo.has_commit(&base.source.commit) {
            return Err(format!(
                "the repository does not have the commit of {} ({}); fetch the release tags",
                base.release, base.source.commit
            ));
        }
    }

    // Which board files to read, and which of their lines.
    let mut fork_symbols = BTreeSet::new();
    let mut boards: Vec<(String, Option<Vec<usize>>)> = Vec::new();
    if let Some(base) = base {
        for file in repo.changed_files(&base.source.commit)? {
            let lines = match file.change {
                Change::Added => None,
                Change::Modified => Some(repo.added_lines(&base.source.commit, &file.path)?),
            };
            if is_kconfig(&file.path) {
                let content = repo.show_head(&file.path)?;
                let defs = kconfig_definitions_in(&content, lines.as_deref());
                fork_symbols.extend(defs);
            } else if !options.all_files && board_of(&file.path).is_some() {
                boards.push((file.path, lines));
            }
        }
    }
    if options.all_files {
        boards = repo
            .head_files()?
            .into_iter()
            .filter(|path| board_of(path).is_some())
            .map(|path| (path, None))
            .collect();
    }

    let mut findings = Vec::new();
    for (path, lines) in &boards {
        let board = board_of(path);
        let content = repo.show_head(path)?;
        for assignment in board_assignments(&content) {
            if lines
                .as_ref()
                .is_some_and(|lines| !lines.contains(&assignment.line))
            {
                continue;
            }
            let symbol = assignment.symbol.as_str();
            if target.defines_symbol(symbol, board.as_deref()) || fork_symbols.contains(symbol) {
                continue;
            }
            let defined_at_base = base.map(|b| b.defines_symbol(symbol, board.as_deref()));
            let message = match (base, defined_at_base) {
                (Some(base), Some(true)) => format!(
                    "{symbol} is not defined in {}; {} defines it, so it was removed or renamed in between",
                    target.release, base.release
                ),
                (Some(base), _) => format!(
                    "{symbol} is not defined in {} nor in {}",
                    target.release, base.release
                ),
                _ => format!("{symbol} is not defined in {}", target.release),
            };
            findings.push(Finding {
                rule: RULE_KCONFIG_UNDEFINED,
                file: path.clone(),
                line: assignment.line,
                symbol: symbol.to_string(),
                text: assignment.text,
                defined_at_base,
                message,
            });
        }
    }
    findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    Ok(Report {
        base: base.map(|b| b.release.clone()),
        target: target.release.clone(),
        head: repo.head()?,
        scope: if options.all_files {
            "all-files"
        } else {
            "changed-files"
        },
        files_checked: boards.len(),
        findings,
    })
}

fn is_kconfig(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|name| name.starts_with("Kconfig"))
}

/// Definitions in a Kconfig file, limited to the given lines when there are any.
fn kconfig_definitions_in(content: &str, lines: Option<&[usize]>) -> Vec<String> {
    match lines {
        None => kconfig_definitions(content),
        Some(lines) => content
            .lines()
            .enumerate()
            .filter(|(index, _)| lines.contains(&(index + 1)))
            .flat_map(|(_, line)| kconfig_definitions(line))
            .collect(),
    }
}
