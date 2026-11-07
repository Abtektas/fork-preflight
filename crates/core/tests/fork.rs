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
        fork_point: None,
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
        fork_point: None,
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
        fork_point: None,
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
                fork_point: None,
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
            fork_point: None,
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
            fork_point: None,
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
            fork_point: None,
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
        fork_point: None,
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
            fork_point: None,
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
            fork_point: None,
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

fn opts(target: &str, fork_point: Option<&str>) -> Options {
    Options {
        base: None,
        target: target.into(),
        all_files: false,
        fork_point: fork_point.map(String::from),
    }
}

/// A fork of upstream's main branch (seen in the P4 fork corpus). The release commit
/// sits on a release branch, so no release is an ancestor of HEAD. Returns the fork, the
/// release commit, the fork point on main and a later main commit.
fn main_fork() -> (Fork, String, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    write(
        path,
        "ROMFS/px4fmu_common/init.d/rcS",
        "param set-default OLD_UP 1\n",
    );
    commit(path, "upstream");
    git(path, &["checkout", "-q", "-b", "release"]);
    write(path, "release-notes.txt", "1.0\n");
    let release_commit = commit(path, "release v1.0.0");
    git(path, &["checkout", "-q", "main"]);
    write(
        path,
        "ROMFS/px4fmu_common/init.d/rcS",
        "param set-default OLD_UP 1\nparam set-default UP_GONE 1\n",
    );
    let fork_point = commit(path, "upstream main moves on");
    write(path, "later.txt", "x\n");
    let later_main = commit(path, "upstream main, later");
    git(path, &["checkout", "-q", "-b", "fork", &fork_point]);
    write(
        path,
        "src/modules/acme_gyro/acme_gyro_params.c",
        "PARAM_DEFINE_INT32(ACME_TKOFF, 0);\n",
    );
    write(
        path,
        "ROMFS/px4fmu_common/init.d/airframes/9002_acme_gyro",
        "param set-default ACME_TKOFF 1\nparam set-default UP_GONE 2\n",
    );
    commit(path, "fork work");
    let repo = Repo::open(path).unwrap();
    (
        Fork {
            _dir: dir,
            repo,
            base: release_commit.clone(),
        },
        release_commit,
        fork_point,
        later_main,
    )
}

fn main_releases(release_commit: &str, main_commit: &str) -> BTreeMap<String, Release> {
    [
        release_with(
            "v1.0.0",
            release_commit,
            &[],
            Some((&["OLD_UP", "UP_GONE"], &[])),
        ),
        release_with(
            "v1.1.0",
            "0000000000000000000000000000000000000001",
            &[],
            Some((&["OLD_UP"], &[])),
        ),
        release_with("main", main_commit, &[], None),
    ]
    .into_iter()
    .collect()
}

#[test]
fn fork_of_main_starts_at_the_merge_base_with_main() {
    let (f, release_commit, fork_point, later_main) = main_fork();
    let report = check::run(
        &f.repo,
        &main_releases(&release_commit, &later_main),
        &opts("v1.1.0", None),
    )
    .unwrap();
    assert_eq!(report.base, None);
    let point = report.fork_point.as_ref().unwrap();
    assert_eq!(point.commit, fork_point);
    assert!(point
        .from
        .starts_with("merge-base of HEAD and the data's main"));
    // The fork's own ACME_TKOFF definition counts; upstream's rcS line is not the fork's.
    assert_eq!(
        rule_summary(&report.findings),
        vec![(
            "param-removed",
            "ROMFS/px4fmu_common/init.d/airframes/9002_acme_gyro".into(),
            2,
            "UP_GONE".into(),
            None
        )]
    );
    assert_eq!(report.skipped, ["command-removed: needs a base release"]);
}

#[test]
fn fork_point_option_and_its_errors() {
    let (f, release_commit, fork_point, later_main) = main_fork();
    // Without the main commit in the data, the error names both ways out.
    let mut data = main_releases(&release_commit, &later_main);
    data.remove("main");
    let err = check::run(&f.repo, &data, &opts("v1.1.0", None)).unwrap_err();
    assert!(
        err.contains("--base") && err.contains("--fork-point"),
        "{err}"
    );
    // A main commit the repository does not have: the error says to fetch it.
    let missing = main_releases(&release_commit, "3333333333333333333333333333333333333333");
    let err = check::run(&f.repo, &missing, &opts("v1.1.0", None)).unwrap_err();
    assert!(err.contains("fetch PX4's main branch"), "{err}");
    // --fork-point takes the fork point itself, or the upstream branch the fork left: its
    // tip is past the fork point, and the change set must not take upstream's later work.
    for rev in ["main~1", "main"] {
        let report = check::run(&f.repo, &missing, &opts("v1.1.0", Some(rev))).unwrap();
        let point = report.fork_point.unwrap();
        assert_eq!(
            (point.commit.as_str(), point.from.as_str()),
            (
                fork_point.as_str(),
                format!("merge-base of HEAD and --fork-point {rev}").as_str()
            )
        );
        assert_eq!(report.findings.len(), 1, "{rev}");
    }
    let err = check::run(&f.repo, &missing, &opts("v1.1.0", Some("no-such-rev"))).unwrap_err();
    assert!(err.contains("no commit no-such-rev"), "{err}");
    let err = check::run(&f.repo, &missing, &opts("v1.1.0", Some("--all"))).unwrap_err();
    assert!(err.contains("not a revision"), "{err}");
    let path = f.repo_path();
    git(&path, &["checkout", "-q", "--orphan", "unrelated"]);
    write(&path, "other.txt", "x\n");
    let unrelated = commit(&path, "unrelated root");
    git(&path, &["checkout", "-q", "fork"]);
    let err = check::run(&f.repo, &missing, &opts("v1.1.0", Some(&unrelated))).unwrap_err();
    assert!(err.contains("no history in common with HEAD"), "{err}");
    // With a release base, the fork point still overrides where the change set starts.
    let report = check::run(
        &f.repo,
        &missing,
        &Options {
            base: Some("v1.0.0".into()),
            ..opts("v1.1.0", Some(&release_commit))
        },
    )
    .unwrap();
    assert_eq!(report.base.as_deref(), Some("v1.0.0"));
    // From the release commit, upstream main's own UP_GONE line in rcS is in the change set too.
    let symbols: Vec<(&str, Option<bool>)> = report
        .findings
        .iter()
        .map(|f| (f.file.as_str(), f.defined_at_base))
        .collect();
    assert_eq!(
        symbols,
        [
            (
                "ROMFS/px4fmu_common/init.d/airframes/9002_acme_gyro",
                Some(true)
            ),
            ("ROMFS/px4fmu_common/init.d/rcS", Some(true)),
        ]
    );
}

#[test]
fn fork_of_a_main_mirror_has_no_changes() {
    // A mirror of upstream main (P4 corpus controls): HEAD is an ancestor of upstream main.
    let (f, release_commit, fork_point, later_main) = main_fork();
    git(&f.repo_path(), &["checkout", "-q", "--detach", &fork_point]);
    let report = check::run(
        &f.repo,
        &main_releases(&release_commit, &later_main),
        &opts("v1.1.0", None),
    )
    .unwrap();
    assert_eq!(report.fork_point.unwrap().commit, fork_point);
    assert_eq!((report.files_checked, report.findings.len()), (0, 0));
}

#[test]
fn names_defined_by_upstream_lines_of_a_fork_modified_file_get_a_hint() {
    // Seen in the P4 corpus: the fork extends an upstream module's parameter file that the
    // target removes, and its airframes set the module's upstream parameters.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "-q"]);
    write(
        path,
        "src/modules/rover/rover_params.c",
        "PARAM_DEFINE_FLOAT(GND_L1_DIST, 5.0f);\n",
    );
    write(
        path,
        "src/modules/rover/Kconfig",
        "menuconfig MODULES_ROVER\n\tbool \"r\"\n",
    );
    write(
        path,
        "src/modules/rover/CMakeLists.txt",
        "px4_add_module(\n\tMODULE rover\n\tMAIN rover_ctl\n)\n",
    );
    let base = commit(path, "upstream release");
    write(
        path,
        "src/modules/rover/rover_params.c",
        "PARAM_DEFINE_FLOAT(GND_L1_DIST, 5.0f);\nPARAM_DEFINE_INT32(GND_VEL_CTRL, 0);\n",
    );
    write(
        path,
        "src/modules/rover/Kconfig",
        "menuconfig MODULES_ROVER\n\tbool \"rover\"\n",
    );
    write(
        path,
        "src/modules/rover/CMakeLists.txt",
        "px4_add_module(\n\tMODULE rover\n\tMAIN rover_ctl\n\tSRCS r.cpp\n)\n",
    );
    write(
        path,
        "ROMFS/px4fmu_common/init.d/airframes/50005_rover",
        "param set-default GND_L1_DIST 5\nparam set-default GND_VEL_CTRL 1\nrover_ctl start\n",
    );
    write(
        path,
        "boards/acme/r1/default.px4board",
        "CONFIG_MODULES_ROVER=y\n",
    );
    commit(path, "fork work");
    let repo = Repo::open(path).unwrap();
    let mut data: BTreeMap<String, Release> = [
        release_with(
            "v1.0.0",
            &base,
            &[("rover_ctl", "src/modules/rover/CMakeLists.txt")],
            Some((&["GND_L1_DIST"], &[("X", "removed", &[])])),
        ),
        release_with(
            "v1.1.0",
            "0000000000000000000000000000000000000001",
            &[],
            Some((&[], &[("GND_L1_DIST", "removed", &[])])),
        ),
    ]
    .into_iter()
    .collect();
    data.get_mut("v1.0.0")
        .unwrap()
        .kconfig
        .symbols
        .insert("MODULES_ROVER".into());
    let report = check::run(&repo, &data, &opts("v1.1.0", None)).unwrap();
    let got: Vec<(&str, &str, Option<&str>)> = report
        .findings
        .iter()
        .map(|f| (f.rule, f.symbol.as_str(), f.hint.as_deref()))
        .collect();
    let kept = |file: &str| {
        format!("the fork modified {file}, which defines it in lines the fork did not change; it stays defined only if the fork keeps that definition through the merge")
    };
    // GND_VEL_CTRL is the fork's own (added line) and not reported.
    assert_eq!(
        got,
        vec![
            (
                "param-removed",
                "GND_L1_DIST",
                Some(
                    format!(
                        "param-lineage: removed in v1.1.0 (removed); {}",
                        kept("src/modules/rover/rover_params.c")
                    )
                    .as_str()
                )
            ),
            (
                "command-removed",
                "rover_ctl",
                Some(
                    format!(
                        "v1.0.0 declares it in src/modules/rover/CMakeLists.txt; {}",
                        kept("src/modules/rover/CMakeLists.txt")
                    )
                    .as_str()
                )
            ),
            (
                "kconfig-undefined",
                "MODULES_ROVER",
                Some(kept("src/modules/rover/Kconfig").as_str())
            ),
        ]
    );
}
