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
