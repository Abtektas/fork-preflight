// SPDX-License-Identifier: BSD-3-Clause
//! Release selection, the fork's change set, and the rules.

use crate::data::{KconfigRemoval, Release, Version};
use crate::git::{Change, Repo};
use crate::parse::{
    board_assignments, board_of, cmake_main_commands, is_script, kconfig_definitions, param_uses,
    script_commands, ParamDefinitions, ParamUse, ScriptCommand,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_KCONFIG_UNDEFINED: &str = "kconfig-undefined";
pub const RULE_PARAM_REMOVED: &str = "param-removed";
pub const RULE_COMMAND_REMOVED: &str = "command-removed";

/// Every rule, with a one-line description (also used in SARIF).
pub const RULES: &[(&str, &str)] = &[
    (
        RULE_KCONFIG_UNDEFINED,
        "A board file assigns a Kconfig symbol that the target release does not define.",
    ),
    (
        RULE_PARAM_REMOVED,
        "A script names a parameter that the target release does not have.",
    ),
    (
        RULE_COMMAND_REMOVED,
        "A script runs a command that the base release declares and the target release does not.",
    ),
];

pub struct Options {
    /// Release the fork is based on; detected from HEAD's history when not given.
    pub base: Option<String>,
    /// Release the fork will merge.
    pub target: String,
    /// Check every board file and script in HEAD instead of the files the fork changed.
    pub all_files: bool,
    /// Commit where the fork left upstream; the change set starts there instead of at the base release.
    pub fork_point: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub base: Option<String>,
    pub target: String,
    pub head: String,
    pub scope: &'static str,
    /// Where the change set starts; null without one (`--all-files` without a base).
    pub fork_point: Option<ForkPoint>,
    pub files_checked: usize,
    /// Rules that could not run, and why.
    pub skipped: Vec<String>,
    pub findings: Vec<Finding>,
}

/// The commit the change set starts at, and how it was chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForkPoint {
    pub commit: String,
    pub from: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub rule: &'static str,
    pub file: String,
    pub line: usize,
    pub symbol: String,
    pub text: String,
    /// Whether the base release defines the symbol; without a base release, for a Kconfig
    /// symbol, whether the fork point's tree does. Null when neither tells.
    pub defined_at_base: Option<bool>,
    pub message: String,
    /// What the data knows about a successor, when it knows something.
    pub hint: Option<String>,
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
        None => detect_base(repo, releases).map(String::from),
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
        if options.fork_point.is_none() && !repo.has_commit(&base.source.commit) {
            return Err(format!(
                "the repository does not have the commit of {} ({}); fetch the release tags",
                base.release, base.source.commit
            ));
        }
    }
    let fork_point = choose_fork_point(repo, releases, base, options)?;
    if let Some(point) = &fork_point {
        fork_point_in_target(repo, releases, target, point)?;
    }

    // Which files to read, and which of their lines; definitions the fork adds.
    let mut fork = ForkDefinitions::default();
    let mut boards: Vec<(String, Option<Vec<usize>>)> = Vec::new();
    let mut scripts: Vec<(String, Option<Vec<usize>>)> = Vec::new();
    if let Some(point) = &fork_point {
        let changed = repo.changed_files(&point.commit)?;
        let mut added = if changed.iter().any(|f| f.change == Change::Modified) {
            repo.added_lines(&point.commit)?
        } else {
            BTreeMap::new()
        };
        let mut definitions: Vec<(String, Option<Vec<usize>>)> = Vec::new();
        for file in changed {
            let kind = if ForkDefinitions::reads(&file.path) {
                &mut definitions
            } else if options.all_files {
                continue;
            } else if board_of(&file.path).is_some() {
                &mut boards
            } else if is_script(&file.path) {
                &mut scripts
            } else {
                continue;
            };
            let lines = match file.change {
                Change::Added => None,
                // A modified text file has hunks; one without (binary) has no lines to check.
                Change::Modified => Some(added.remove(&file.path).unwrap_or_default()),
            };
            kind.push((file.path, lines));
        }
        let paths: Vec<&str> = definitions.iter().map(|(p, _)| p.as_str()).collect();
        for ((path, lines), content) in definitions.iter().zip(repo.show_head_many(&paths)?) {
            fork.add_file(path, &content, lines.as_deref());
        }
    }
    if options.all_files {
        for path in repo.head_files()? {
            if board_of(&path).is_some() {
                boards.push((path, None));
            } else if is_script(&path) {
                scripts.push((path, None));
            }
        }
    }

    let mut skipped = Vec::new();
    if target.parameters.is_none() {
        skipped.push(format!(
            "{RULE_PARAM_REMOVED}: the data has no parameters for {}",
            target.release
        ));
    }
    if base.is_none() {
        skipped.push(format!("{RULE_COMMAND_REMOVED}: needs a base release"));
    }

    // Without a base release, Kconfig definitions come from the fork point's own tree.
    let fork_point_tree = match (&fork_point, base) {
        (Some(point), None) if !boards.is_empty() => Some(TreeSymbols::read(repo, &point.commit)?),
        _ => None,
    };
    let context = Context {
        base,
        target,
        releases,
        fork: &fork,
        fork_point_tree: fork_point_tree.as_ref(),
    };
    let mut findings = Vec::new();
    let paths: Vec<&str> = boards.iter().map(|(p, _)| p.as_str()).collect();
    for ((path, lines), content) in boards.iter().zip(repo.show_head_many(&paths)?) {
        findings.extend(context.board_findings(path, &content, lines.as_deref()));
    }
    let paths: Vec<&str> = scripts.iter().map(|(p, _)| p.as_str()).collect();
    for ((path, lines), content) in scripts.iter().zip(repo.show_head_many(&paths)?) {
        findings.extend(context.script_findings(path, &content, lines.as_deref()));
    }
    findings.sort_by(|a, b| (&a.file, a.line, a.rule).cmp(&(&b.file, b.line, b.rule)));
    Ok(Report {
        base: base.map(|b| b.release.clone()),
        target: target.release.clone(),
        head: repo.head()?,
        scope: if options.all_files {
            "all-files"
        } else {
            "changed-files"
        },
        fork_point,
        files_checked: boards.len() + scripts.len(),
        skipped,
        findings,
    })
}

/// Where the change set starts: `--fork-point`, else the base release's commit, else the
/// merge-base of HEAD and the data's `main` commit (a fork of PX4's main branch).
fn choose_fork_point(
    repo: &Repo,
    releases: &BTreeMap<String, Release>,
    base: Option<&Release>,
    options: &Options,
) -> Result<Option<ForkPoint>, String> {
    if let Some(rev) = &options.fork_point {
        // The fork point itself, or an upstream branch the fork left (its tip is past the fork point).
        let commit = repo.resolve_commit(rev)?;
        let commit = repo
            .merge_base_with_head(&commit)
            .ok_or_else(|| format!("--fork-point {rev} has no history in common with HEAD"))?;
        return Ok(Some(ForkPoint {
            commit,
            from: format!("merge-base of HEAD and --fork-point {rev}"),
        }));
    }
    if let Some(base) = base {
        return Ok(Some(ForkPoint {
            commit: base.source.commit.clone(),
            from: format!("base release {}", base.release),
        }));
    }
    let main = releases.get("main").map(|m| m.source.commit.as_str());
    if let Some(main) = main.filter(|commit| repo.has_commit(commit)) {
        if let Some(commit) = repo.merge_base_with_head(main) {
            return Ok(Some(ForkPoint {
                commit,
                from: format!(
                    "merge-base of HEAD and the data's main {}",
                    &main[..main.len().min(10)]
                ),
            }));
        }
    }
    if options.all_files {
        return Ok(None);
    }
    let main_hint = main
        .map(|commit| format!(", or fetch PX4's main branch (the data's main is {commit})"))
        .unwrap_or_default();
    Err(format!(
        "no release in the data is an ancestor of HEAD; name the base with --base, the commit where the fork left upstream with --fork-point{main_hint}"
    ))
}

/// A fork point on upstream main that the target does not contain: the fork left main
/// after the target did (a release branch cut earlier), so it already has upstream work the
/// target lacks, and every name main added since would read as missing in the target.
/// Release-branch fork points are not on main and pass. Skipped when the repository lacks
/// the data's main or target commit.
fn fork_point_in_target(
    repo: &Repo,
    releases: &BTreeMap<String, Release>,
    target: &Release,
    point: &ForkPoint,
) -> Result<(), String> {
    let Some(main) = releases.get("main") else {
        return Ok(());
    };
    let (main, target_commit) = (&main.source.commit, &target.source.commit);
    if main == target_commit || !repo.has_commit(main) || !repo.has_commit(target_commit) {
        return Ok(());
    }
    if repo.is_ancestor(&point.commit, main) && !repo.is_ancestor(&point.commit, target_commit) {
        return Err(format!(
            "the fork point {} is on upstream main after {} left it: the fork already has upstream work that {} lacks, so merging it is not an upgrade; choose a newer target, such as main",
            &point.commit[..point.commit.len().min(10)],
            target.release,
            target.release
        ));
    }
    Ok(())
}

/// Kconfig symbols, commands and parameters that a set of definitions holds.
#[derive(Default)]
struct Defined {
    symbols: BTreeSet<String>,
    commands: BTreeSet<String>,
    params: ParamDefinitions,
}

impl Defined {
    fn add_file(&mut self, path: &str, content: &str, lines: Option<&[usize]>) {
        let name = path.rsplit('/').next().unwrap_or(path);
        if name.starts_with("Kconfig") {
            self.symbols.extend(kconfig_definitions_in(content, lines));
        } else if name == "CMakeLists.txt" {
            self.commands.extend(cmake_main_commands(content, lines));
        } else {
            self.params.add_file(path, content, lines);
        }
    }
}

/// What the fork's own added or changed lines define, and what the whole of each file it
/// modified defines (upstream lines included, which the merge may drop).
#[derive(Default)]
struct ForkDefinitions {
    added: Defined,
    modified_files: Vec<(String, Defined)>,
}

impl ForkDefinitions {
    /// Whether a changed file can define something: Kconfig, CMakeLists.txt, C/C++ or YAML.
    fn reads(path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        name.starts_with("Kconfig")
            || name == "CMakeLists.txt"
            || name
                .rsplit_once('.')
                .is_some_and(|(_, ext)| matches!(ext, "c" | "cpp" | "h" | "hpp" | "yaml" | "yml"))
    }

    fn add_file(&mut self, path: &str, content: &str, lines: Option<&[usize]>) {
        self.added.add_file(path, content, lines);
        if lines.is_some() {
            let mut whole = Defined::default();
            whole.add_file(path, content, None);
            self.modified_files.push((path.to_string(), whole));
        }
    }

    /// The fork-modified files whose upstream lines define a name, for a hint.
    fn kept_hint(&self, defines: impl Fn(&Defined) -> bool) -> Option<String> {
        let files: Vec<&str> = self
            .modified_files
            .iter()
            .filter(|(_, defined)| defines(defined))
            .map(|(path, _)| path.as_str())
            .collect();
        (!files.is_empty()).then(|| {
            format!(
                "the fork modified {}, which defines it in lines the fork did not change; it stays defined only if the fork keeps that definition through the merge",
                files.join(", ")
            )
        })
    }
}

fn join_hints(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(a), Some(b)) => Some(format!("{a}; {b}")),
        (a, b) => a.or(b),
    }
}

struct Context<'a> {
    base: Option<&'a Release>,
    target: &'a Release,
    releases: &'a BTreeMap<String, Release>,
    fork: &'a ForkDefinitions,
    /// Kconfig definitions at the fork point, read when there is no base release.
    fork_point_tree: Option<&'a TreeSymbols>,
}

/// Kconfig symbols that a commit's tree defines, found by text: `config NAME` and
/// `menuconfig NAME` lines in every `Kconfig*` file, a file under `boards/<vendor>/<model>/`
/// counting for that board only. `nuttx-config/` files hold NuttX's board options, not
/// PX4's, and are left out. On every data release this equals the data's kconfiglib set,
/// except for one file no Kconfig sources (`DRIVERS_GHST_RC` of two modalai boards).
struct TreeSymbols {
    commit: String,
    common: BTreeSet<String>,
    boards: BTreeMap<String, BTreeSet<String>>,
    /// The tree generates its Zenoh topic symbols at build time (`$(ZENOH_KCONFIG_TOPICS)`).
    generates_zenoh_topics: bool,
}

const ZENOH_TOPIC_PREFIX: &str = "ZENOH_PUBSUB_";

impl TreeSymbols {
    fn read(repo: &Repo, commit: &str) -> Result<TreeSymbols, String> {
        let files: Vec<String> = repo
            .blob_files(commit)?
            .into_iter()
            .filter(|path| {
                let name = path.rsplit('/').next().unwrap_or(path);
                name.starts_with("Kconfig") && !path.split('/').any(|part| part == "nuttx-config")
            })
            .collect();
        let paths: Vec<&str> = files.iter().map(String::as_str).collect();
        let mut tree = TreeSymbols {
            commit: commit.to_string(),
            common: BTreeSet::new(),
            boards: BTreeMap::new(),
            generates_zenoh_topics: false,
        };
        for (path, content) in files.iter().zip(repo.show_many(commit, &paths)?) {
            tree.generates_zenoh_topics |= content.contains("$(ZENOH_KCONFIG_TOPICS)");
            let parts: Vec<&str> = path.split('/').collect();
            let symbols = match parts.as_slice() {
                ["boards", vendor, model, _, ..] => {
                    tree.boards.entry(format!("{vendor}/{model}")).or_default()
                }
                _ => &mut tree.common,
            };
            symbols.extend(
                kconfig_definitions(&content)
                    .into_iter()
                    .filter(|name| name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')),
            );
        }
        Ok(tree)
    }

    /// Whether the tree defines a symbol; None for a Zenoh topic symbol of a tree that
    /// generates them.
    fn defines(&self, symbol: &str, board: Option<&str>) -> Option<bool> {
        let found = self.common.contains(symbol)
            || board
                .and_then(|board| self.boards.get(board))
                .is_some_and(|symbols| symbols.contains(symbol));
        if !found && self.generates_zenoh_topics && symbol.starts_with(ZENOH_TOPIC_PREFIX) {
            return None;
        }
        Some(found)
    }
}

/// Whether a name is `<stem><n>` (n >= 1) of a family whose `<stem>1` the data has, such as
/// `PWM_MAIN_FUNC9` when the data lists `PWM_MAIN_FUNC1` to `8`.
fn is_numbered_instance(name: &str, names: &BTreeSet<String>) -> bool {
    let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let number = &name[stem.len()..];
    !stem.is_empty()
        && !number.is_empty()
        && !number.starts_with('0')
        && names.contains(&format!("{stem}1"))
}

fn wanted(lines: Option<&[usize]>, line: usize) -> bool {
    lines.is_none_or(|lines| lines.contains(&line))
}

impl Context<'_> {
    fn board_findings(&self, path: &str, content: &str, lines: Option<&[usize]>) -> Vec<Finding> {
        let (base, target) = (self.base, self.target);
        let board = board_of(path);
        let mut findings = Vec::new();
        for assignment in board_assignments(content) {
            if !wanted(lines, assignment.line) {
                continue;
            }
            let symbol = assignment.symbol.as_str();
            if target.defines_symbol(symbol, board.as_deref())
                || self.fork.added.symbols.contains(symbol)
            {
                continue;
            }
            // What the definitions were checked against, and the word before it.
            let (defined_at_base, at) = match (base, self.fork_point_tree) {
                (Some(base), _) => (
                    Some(base.defines_symbol(symbol, board.as_deref())),
                    Some(("in", base.release.clone())),
                ),
                (None, Some(tree)) => (
                    tree.defines(symbol, board.as_deref()),
                    Some((
                        "at",
                        format!(
                            "the fork point {}",
                            &tree.commit[..tree.commit.len().min(10)]
                        ),
                    )),
                ),
                (None, None) => (None, None),
            };
            let message = match (at, defined_at_base) {
                (Some((_, at)), Some(true)) => format!(
                    "{symbol} is not defined in {}; {at} defines it, so it was removed or renamed in between",
                    target.release
                ),
                (Some((word, at)), Some(false)) => format!(
                    "{symbol} is not defined in {} nor {word} {at}",
                    target.release
                ),
                _ => format!("{symbol} is not defined in {}", target.release),
            };
            findings.push(Finding {
                rule: RULE_KCONFIG_UNDEFINED,
                file: path.to_string(),
                line: assignment.line,
                symbol: symbol.to_string(),
                text: assignment.text,
                defined_at_base,
                message,
                hint: join_hints(
                    if defined_at_base == Some(true) {
                        self.kconfig_removal_hint(symbol, board.as_deref())
                    } else {
                        None
                    },
                    self.fork.kept_hint(|d| d.symbols.contains(symbol)),
                ),
            });
        }
        findings
    }

    fn script_findings(&self, path: &str, content: &str, lines: Option<&[usize]>) -> Vec<Finding> {
        let mut findings = Vec::new();
        for command in script_commands(content) {
            if !wanted(lines, command.line) {
                continue;
            }
            if let Some(finding) = self.command_finding(path, &command) {
                findings.push(finding);
            }
            for used in param_uses(&command) {
                if let Some(finding) = self.param_finding(path, &command, &used) {
                    findings.push(finding);
                }
            }
        }
        findings
    }

    fn command_finding(&self, path: &str, command: &ScriptCommand) -> Option<Finding> {
        let base = self.base?;
        let name = command.command.as_str();
        let declared_at_base = base.commands.get(name)?;
        if self.target.commands.contains_key(name) || self.fork.added.commands.contains(name) {
            return None;
        }
        let mut hint = format!(
            "{} declares it in {}",
            base.release,
            declared_at_base.join(", ")
        );
        let successors: BTreeSet<&str> = self
            .target
            .commands
            .iter()
            // Only module files: apps.cpp.in declares every POSIX builtin, not a successor.
            .filter(|(_, files)| {
                files
                    .iter()
                    .any(|f| f.ends_with("CMakeLists.txt") && declared_at_base.contains(f))
            })
            .map(|(other, _)| other.as_str())
            .collect();
        if !successors.is_empty() {
            let list: Vec<&str> = successors.into_iter().collect();
            hint.push_str(&format!(
                "; in {} that file declares {}",
                self.target.release,
                list.join(", ")
            ));
        }
        Some(Finding {
            rule: RULE_COMMAND_REMOVED,
            file: path.to_string(),
            line: command.line,
            symbol: name.to_string(),
            text: command.text.clone(),
            defined_at_base: Some(true),
            message: format!(
                "{name} is not a command in {}; {} has it, so its module was removed or renamed in between",
                self.target.release, base.release
            ),
            hint: join_hints(
                Some(hint),
                self.fork.kept_hint(|d| d.commands.contains(name)),
            ),
        })
    }

    fn param_finding(
        &self,
        path: &str,
        command: &ScriptCommand,
        used: &ParamUse,
    ) -> Option<Finding> {
        let target_params = self.target.parameters.as_ref()?;
        let name = used.name.as_str();
        if target_params.names.contains(name) || self.fork.added.params.contains(name) {
            return None;
        }
        let defined_at_base = self
            .base
            .and_then(|b| b.parameters.as_ref())
            .map(|p| p.names.contains(name));
        if defined_at_base != Some(true) && is_numbered_instance(name, &target_params.names) {
            // The number of instances depends on the board (output channels); the data has one count.
            return None;
        }
        let target = &self.target.release;
        let mut message = match (self.base, defined_at_base) {
            (Some(base), Some(true)) => format!(
                "{name} is not a parameter in {target}; {} has it, so it was removed or renamed in between",
                base.release
            ),
            (Some(base), Some(false)) => format!(
                "{name} is not in the parameter data of {target} nor of {}",
                base.release
            ),
            (Some(base), None) => format!(
                "{name} is not a parameter in {target}; the data has no parameters for {}, so it may already be missing there",
                base.release
            ),
            _ => format!("{name} is not a parameter in {target}"),
        };
        if used.silent {
            message.push_str(&format!(
                " (`param {} -s` hides PX4's error for it)",
                used.subcommand
            ));
        }
        Some(Finding {
            rule: RULE_PARAM_REMOVED,
            file: path.to_string(),
            line: command.line,
            symbol: name.to_string(),
            text: command.text.clone(),
            defined_at_base,
            message,
            hint: join_hints(
                self.removal_hint(name),
                self.fork.kept_hint(|d| d.params.contains(name)),
            ),
        })
    }

    /// Data releases after the base, up to the target, oldest first.
    fn releases_in_range(&self) -> Vec<&Release> {
        let low = self.base.map(|b| Version::parse(&b.release));
        let high = Version::parse(&self.target.release);
        let mut in_range: Vec<&Release> = self
            .releases
            .values()
            .filter(|r| {
                let v = Version::parse(&r.release);
                v <= high && low.as_ref().is_none_or(|low| v > *low)
            })
            .collect();
        in_range.sort_by_key(|r| Version::parse(&r.release));
        in_range
    }

    /// What PX4's history says about the latest removal of a Kconfig symbol after the base,
    /// up to the target, following its successors through later releases to the target.
    fn kconfig_removal_hint(&self, symbol: &str, board: Option<&str>) -> Option<String> {
        let in_range = self.releases_in_range();
        let (index, removal) = kconfig_removal_in(&in_range, symbol, 0)?;
        let release = &in_range[index].release;
        let mut hint = format!("removed in {release}");
        if !removal.removed_by.is_empty() {
            let commits: Vec<String> = removal
                .removed_by
                .iter()
                .map(|c| format!("{} ({})", &c.commit[..c.commit.len().min(10)], c.subject))
                .collect();
            hint.push_str(&format!(" by {}", commits.join(", ")));
        }
        let Some(words) = successor_words(removal) else {
            return Some(hint);
        };
        hint.push_str(&format!("; {words}"));
        let mut steps = Vec::new();
        let mut defined = Vec::new();
        for successor in &removal.successors {
            self.follow(
                &in_range,
                successor,
                index + 1,
                board,
                0,
                &mut steps,
                &mut defined,
            );
        }
        if !steps.is_empty() {
            hint.push_str(&format!("; then {}", steps.join(", ")));
        }
        if defined.is_empty() {
            hint.push_str(&format!("; {} defines none of them", self.target.release));
        }
        Some(hint)
    }

    /// Follow a successor named in an earlier release to the names the target defines.
    #[allow(clippy::too_many_arguments)]
    fn follow(
        &self,
        in_range: &[&Release],
        name: &str,
        from: usize,
        board: Option<&str>,
        depth: usize,
        steps: &mut Vec<String>,
        defined: &mut Vec<String>,
    ) {
        if self.target.defines_symbol(name, board) {
            if !defined.iter().any(|d| d == name) {
                defined.push(name.to_string());
            }
            return;
        }
        if depth >= MAX_FOLLOW {
            return;
        }
        let Some((index, removal)) = kconfig_removal_in(in_range, name, from) else {
            return;
        };
        let release = &in_range[index].release;
        match successor_words(removal) {
            Some(words) => steps.push(format!("in {release} {name}: {words}")),
            None => steps.push(format!("{name} was removed in {release}")),
        }
        for successor in &removal.successors {
            self.follow(
                in_range,
                successor,
                index + 1,
                board,
                depth + 1,
                steps,
                defined,
            );
        }
    }

    /// What param-lineage says about the latest removal of a parameter after the base, up to the target.
    fn removal_hint(&self, name: &str) -> Option<String> {
        let in_range = self.releases_in_range();
        let (release, removal) = in_range.iter().rev().find_map(|r| {
            r.parameters
                .as_ref()
                .and_then(|p| p.removed.get(name))
                .map(|removal| (r.release.as_str(), removal))
        })?;
        let mut hint = format!("param-lineage: removed in {release}");
        if let Some(kind) = &removal.kind {
            hint.push_str(&format!(" ({kind}"));
            match kind.as_str() {
                "migrated" => hint.push_str(", PX4 converts a stored value"),
                "renamed" => hint.push_str(", PX4 does not convert a stored value"),
                _ => {}
            }
            hint.push(')');
        }
        if !removal.successors.is_empty() {
            hint.push_str(&format!("; successors: {}", removal.successors.join(", ")));
        }
        Some(hint)
    }
}

/// How many releases a successor is followed through after the one that removed the symbol.
const MAX_FOLLOW: usize = 4;

/// The latest removal of a symbol among `in_range[from..]`, with its index.
fn kconfig_removal_in<'a>(
    in_range: &[&'a Release],
    symbol: &str,
    from: usize,
) -> Option<(usize, &'a KconfigRemoval)> {
    in_range
        .iter()
        .enumerate()
        .skip(from)
        .rev()
        .find_map(|(index, r)| {
            r.kconfig
                .removed
                .as_ref()
                .and_then(|removed| removed.symbols.get(symbol))
                .map(|removal| (index, removal))
        })
}

/// The evidence for a removal's successors in words, or None when it names none.
fn successor_words(removal: &KconfigRemoval) -> Option<String> {
    if removal.successors.is_empty() {
        return None;
    }
    let names = removal.successors.join(", ");
    Some(match removal.evidence.as_deref() {
        Some("renamed") if !removal.via.is_empty() => {
            format!("renamed to {names} (via {})", removal.via.join(", "))
        }
        Some("renamed") => format!("renamed to {names}"),
        Some("boards") => {
            let counted: Vec<String> = removal
                .successors
                .iter()
                .map(|s| match removal.boards.get(s) {
                    Some(count) => format!("{s} ({}/{})", count.boards, count.of),
                    None => s.clone(),
                })
                .collect();
            format!("in-tree boards set {} instead", counted.join(" and "))
        }
        Some("selected") => format!("it selected {names}"),
        Some("named") => format!("its commit message names {names}"),
        _ => format!("successors: {names}"),
    })
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
