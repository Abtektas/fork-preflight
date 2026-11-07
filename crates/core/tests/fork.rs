// SPDX-License-Identifier: BSD-3-Clause
//! The scanner on small synthetic fork repositories.

use fork_preflight_core::check::{self, Finding, Options};
use fork_preflight_core::data::Release;
use fork_preflight_core::git::Repo;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

fn release(
    name: &str,
    commit: &str,
    symbols: &[&str],
    boards: &[(&str, &[&str])],
) -> (String, Release) {
    let value = serde_json::json!({
        "format": 1,
        "release": name,
        "source": {"commit": commit},
        "kconfig": {
            "symbols": symbols,
            "board_symbols": boards.iter().map(|(b, s)| (b.to_string(), s.to_vec())).collect::<BTreeMap<_, _>>(),
        },
        "commands": {},
        "parameters": null,
    });
    (name.to_string(), serde_json::from_value(value).unwrap())
}

struct Fork {
    _dir: tempfile::TempDir,
    repo: Repo,
    base: String,
}

/// Base: an upstream board file. Fork: changes it, adds a board and a Kconfig file.
fn fork() -> Fork {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "-q"]);
    write(
        path,
        "boards/up/one/default.px4board",
        "CONFIG_OLD=y\nCONFIG_KEEP=y\n# CONFIG_OLD_OFF is not set\n",
    );
    write(path, "Kconfig", "config KEEP\n\tbool \"k\"\n");
    let base = commit(path, "upstream release");
    write(
        path,
        "boards/up/one/default.px4board",
        "CONFIG_OLD=y\nCONFIG_KEEP=y\nCONFIG_FORK_ADDED=y\n# CONFIG_OLD_OFF is not set\nCONFIG_NEWMOD=y\n",
    );
    write(
        path,
        "boards/acme/x1/default.px4board",
        "CONFIG_GONE=y\nCONFIG_TYPO=y\nCONFIG_FORK_SYM=y\nCONFIG_BOARD_ONLY=y\n",
    );
    write(
        path,
        "src/modules/acme/Kconfig",
        "menuconfig FORK_SYM\n\tbool \"f\"\n",
    );
    write(path, "README.md", "fork\n");
    commit(path, "fork work");
    let repo = Repo::open(path).unwrap();
    Fork {
        _dir: dir,
        repo,
        base,
    }
}

fn releases(base: &str) -> BTreeMap<String, Release> {
    [
        release("v1.0.0", base, &["OLD", "OLD_OFF", "KEEP", "GONE"], &[]),
        release(
            "v1.1.0",
            "0000000000000000000000000000000000000000",
            &["KEEP", "NEWMOD"],
            &[("acme/x1", &["BOARD_ONLY"])],
        ),
    ]
    .into_iter()
    .collect()
}

fn summary(findings: &[Finding]) -> Vec<(String, usize, String, Option<bool>)> {
    findings
        .iter()
        .map(|f| (f.file.clone(), f.line, f.symbol.clone(), f.defined_at_base))
        .collect()
}

#[test]
fn changed_lines_and_added_files_only() {
    let f = fork();
    let options = Options {
        base: None,
        target: "v1.1.0".into(),
        all_files: false,
    };
    let report = check::run(&f.repo, &releases(&f.base), &options).unwrap();
    assert_eq!(report.base.as_deref(), Some("v1.0.0"));
    assert_eq!(report.scope, "changed-files");
    assert_eq!(report.files_checked, 2);
    assert_eq!(
        summary(&report.findings),
        vec![
            (
                "boards/acme/x1/default.px4board".into(),
                1,
                "GONE".into(),
                Some(true)
            ),
            (
                "boards/acme/x1/default.px4board".into(),
                2,
                "TYPO".into(),
                Some(false)
            ),
            (
                "boards/up/one/default.px4board".into(),
                3,
                "FORK_ADDED".into(),
                Some(false)
            ),
        ]
    );
    assert!(report.findings[0].message.contains("v1.0.0 defines it"));
    assert!(report.findings[1].message.contains("nor in v1.0.0"));
}

#[test]
fn all_files_also_reports_upstream_lines() {
    let f = fork();
    let options = Options {
        base: None,
        target: "v1.1.0".into(),
        all_files: true,
    };
    let report = check::run(&f.repo, &releases(&f.base), &options).unwrap();
    let symbols: Vec<String> = report.findings.iter().map(|f| f.symbol.clone()).collect();
    assert_eq!(symbols, ["GONE", "TYPO", "OLD", "FORK_ADDED", "OLD_OFF"]);
}

#[test]
fn all_files_without_any_known_base() {
    let f = fork();
    let mut data = releases(&f.base);
    data.remove("v1.0.0");
    let options = Options {
        base: None,
        target: "v1.1.0".into(),
        all_files: true,
    };
    let report = check::run(&f.repo, &data, &options).unwrap();
    assert_eq!(report.base, None);
    // Without a base there is no change set, so fork Kconfig files do not count.
    assert!(report
        .findings
        .iter()
        .any(|f| f.symbol == "FORK_SYM" && f.defined_at_base.is_none()));
}

#[test]
fn errors_are_clear() {
    let f = fork();
    let data = releases(&f.base);
    let run = |base: Option<&str>, target: &str| {
        check::run(
            &f.repo,
            &data,
            &Options {
                base: base.map(String::from),
                target: target.into(),
                all_files: false,
            },
        )
        .unwrap_err()
    };
    assert!(run(None, "v9.9.9").contains("no data for release v9.9.9"));
    assert!(run(Some("v1.1.0"), "v1.0.0").contains("not newer than base"));
    assert!(run(Some("v1.1.0"), "v1.1.0").contains("not newer than base"));

    let mut unrelated = releases("1111111111111111111111111111111111111111");
    unrelated.extend([release(
        "v1.2.0",
        "2222222222222222222222222222222222222222",
        &[],
        &[],
    )]);
    let err = check::run(
        &f.repo,
        &unrelated,
        &Options {
            base: None,
            target: "v1.1.0".into(),
            all_files: false,
        },
    )
    .unwrap_err();
    assert!(err.contains("name the base with --base"), "{err}");
    let err = check::run(
        &f.repo,
        &unrelated,
        &Options {
            base: Some("v1.0.0".into()),
            target: "v1.1.0".into(),
            all_files: false,
        },
    )
    .unwrap_err();
    assert!(err.contains("fetch the release tags"), "{err}");
}

#[test]
fn newest_ancestor_release_is_the_base() {
    let f = fork();
    let older = f.base.clone();
    let path = f.repo_path();
    // A newer release whose commit is HEAD itself must win over v1.0.0.
    let head = git(&path, &["rev-parse", "HEAD"]);
    let mut data = releases(&older);
    data.extend([release(
        "v1.0.1",
        &head,
        &["OLD", "KEEP", "GONE", "TYPO", "FORK_ADDED"],
        &[],
    )]);
    let report = check::run(
        &f.repo,
        &data,
        &Options {
            base: None,
            target: "v1.1.0".into(),
            all_files: false,
        },
    )
    .unwrap();
    assert_eq!(report.base.as_deref(), Some("v1.0.1"));
    // Nothing changed since v1.0.1, so there is nothing to check.
    assert_eq!((report.files_checked, report.findings.len()), (0, 0));
}

impl Fork {
    fn repo_path(&self) -> std::path::PathBuf {
        self._dir.path().to_path_buf()
    }
}

/// A removed parameter: (name, kind, successors).
type Removed<'a> = (&'a str, &'a str, &'a [&'a str]);

/// A release with commands and parameters (names, removed).
fn release_with(
    name: &str,
    commit: &str,
    commands: &[(&str, &str)],
    params: Option<(&[&str], &[Removed])>,
) -> (String, Release) {
    let parameters = params.map(|(names, removed)| {
        serde_json::json!({
            "names": names,
            "removed": removed
                .iter()
                .map(|(n, kind, successors)| {
                    (n.to_string(), serde_json::json!({"kind": kind, "successors": successors}))
                })
                .collect::<BTreeMap<_, _>>(),
        })
    });
    let value = serde_json::json!({
        "format": 1,
        "release": name,
        "source": {"commit": commit},
        "kconfig": {"symbols": [], "board_symbols": {}},
        "commands": commands
            .iter()
            .map(|(c, file)| (c.to_string(), vec![file.to_string()]))
            .collect::<BTreeMap<_, _>>(),
        "parameters": parameters,
    });
    (name.to_string(), serde_json::from_value(value).unwrap())
}

/// Base: upstream rcS and a heater module. Fork: adds lines to rcS, an airframe and its own module.
fn script_fork() -> Fork {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "-q"]);
    write(
        path,
        "ROMFS/px4fmu_common/init.d/rcS",
        "param set-default OLD_UP 1\nheater start\n",
    );
    let base = commit(path, "upstream release");
    write(
        path,
        "ROMFS/px4fmu_common/init.d/rcS",
        "param set-default OLD_UP 1\nheater start\nif param compare -s GONE_P 1\nthen\n\tacme_cmd start\nfi\necho done\n",
    );
    write(
        path,
        "ROMFS/px4fmu_common/init.d/airframes/9001_acme",
        "# param set-default GONE_P 2\nparam set-default GONE_P 2\nparam set-default ACME_EN 1\nparam set-default ACME_2_ID 7\nparam set-default TYPO_P 1\nparam set-default PWM_MAIN_FUNC9 101\nparam set-default KEPT_P 3\nheater stop\n",
    );
    write(
        path,
        "src/drivers/acme/CMakeLists.txt",
        "px4_add_module(\n\tMODULE drivers__acme\n\tMAIN acme_cmd\n)\n",
    );
    write(
        path,
        "src/drivers/acme/acme_params.c",
        "PARAM_DEFINE_INT32(ACME_EN, 0);\n",
    );
    write(
        path,
        "src/drivers/acme/module.yaml",
        "parameters:\n  - definitions:\n      ACME_${i}_ID:\n        num_instances: 3\n",
    );
    commit(path, "fork work");
    let repo = Repo::open(path).unwrap();
    Fork {
        _dir: dir,
        repo,
        base,
    }
}

fn script_releases(base: &str, target_params: bool) -> BTreeMap<String, Release> {
    let target_names: &[&str] = &["OLD_UP", "KEPT_P", "NEW_P", "PWM_MAIN_FUNC1"];
    let removed: &[Removed] = &[("GONE_P", "renamed", &["NEW_P"])];
    [
        release_with(
            "v1.0.0",
            base,
            &[("heater", "src/drivers/heater/CMakeLists.txt")],
            Some((&["OLD_UP", "KEPT_P", "GONE_P", "PWM_MAIN_FUNC1"], &[])),
        ),
        release_with(
            "v1.1.0",
            "0000000000000000000000000000000000000000",
            &[("heater2", "src/drivers/heater/CMakeLists.txt")],
            target_params.then_some((target_names, removed)),
        ),
    ]
    .into_iter()
    .collect()
}

fn rule_summary(findings: &[Finding]) -> Vec<(&'static str, String, usize, String, Option<bool>)> {
    findings
        .iter()
        .map(|f| {
            (
                f.rule,
                f.file.clone(),
                f.line,
                f.symbol.clone(),
                f.defined_at_base,
            )
        })
        .collect()
}

#[test]
fn scripts_parameters_and_commands() {
    let f = script_fork();
    let options = Options {
        base: None,
        target: "v1.1.0".into(),
        all_files: false,
    };
    let report = check::run(&f.repo, &script_releases(&f.base, true), &options).unwrap();
    assert!(report.skipped.is_empty());
    assert_eq!(report.files_checked, 2);
    let airframe = "ROMFS/px4fmu_common/init.d/airframes/9001_acme".to_string();
    let rcs = "ROMFS/px4fmu_common/init.d/rcS".to_string();
    assert_eq!(
        rule_summary(&report.findings),
        vec![
            (
                "param-removed",
                airframe.clone(),
                2,
                "GONE_P".into(),
                Some(true)
            ),
            (
                "param-removed",
                airframe.clone(),
                5,
                "TYPO_P".into(),
                Some(false)
            ),
            (
                "command-removed",
                airframe.clone(),
                8,
                "heater".into(),
                Some(true)
            ),
            // Line 2 of rcS (`heater start`) is upstream's own and not reported.
            ("param-removed", rcs.clone(), 3, "GONE_P".into(), Some(true)),
        ]
    );
    let gone = &report.findings[0];
    assert_eq!(
        gone.hint.as_deref(),
        Some("param-lineage: removed in v1.1.0 (renamed, PX4 does not convert a stored value); successors: NEW_P")
    );
    assert!(report.findings[3]
        .message
        .contains("`param compare -s` hides"));
    assert_eq!(
        report.findings[2].hint.as_deref(),
        Some("v1.0.0 declares it in src/drivers/heater/CMakeLists.txt; in v1.1.0 that file declares heater2")
    );
}

#[test]
fn rules_without_their_data_are_skipped() {
    let f = script_fork();
    let report = check::run(
        &f.repo,
        &script_releases(&f.base, false),
        &Options {
            base: None,
            target: "v1.1.0".into(),
            all_files: false,
        },
    )
    .unwrap();
    assert_eq!(
        report.skipped,
        ["param-removed: the data has no parameters for v1.1.0"]
    );
    assert!(report.findings.iter().all(|f| f.rule == "command-removed"));
    let mut data = script_releases(&f.base, true);
    data.remove("v1.0.0");
    let report = check::run(
        &f.repo,
        &data,
        &Options {
            base: None,
            target: "v1.1.0".into(),
            all_files: true,
        },
    )
    .unwrap();
    assert_eq!(report.skipped, ["command-removed: needs a base release"]);
    // Without a base, the fork's own definitions are unknown, so ACME_EN counts as a finding.
    let symbols: Vec<&str> = report.findings.iter().map(|f| f.symbol.as_str()).collect();
    assert_eq!(
        symbols,
        ["GONE_P", "ACME_EN", "ACME_2_ID", "TYPO_P", "GONE_P"]
    );
    assert!(report.findings.iter().all(|f| f.defined_at_base.is_none()));
}
