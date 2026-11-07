// SPDX-License-Identifier: BSD-3-Clause
//! The binary: notices, exit codes, JSON, and known answers on a PX4 clone.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fork-preflight"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

#[test]
fn version_and_license_carry_the_notices() {
    let out = run(&["--version"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.starts_with("fork-preflight "));
    for phrase in [
        "without warranty of any kind",
        "accept no liability",
        "not affiliated with or endorsed by the PX4 project",
    ] {
        assert!(text.contains(phrase), "{phrase}");
    }
    let text = stdout(&run(&["--license"]));
    assert!(
        text.contains("the fork-preflight contributors") && text.contains("PX4 Development Team")
    );
}

#[test]
fn list_releases() {
    let text = stdout(&run(&["--list-releases"]));
    assert!(text
        .lines()
        .any(|l| l == "v1.17.0 d6f12ad1c4f70ad3230afd7d86e971421e02fef4"));
    assert!(text.lines().any(|l| l.starts_with("main ")));
}

#[test]
fn usage_and_data_errors_exit_2() {
    assert_eq!(run(&[]).status.code(), Some(2));
    let dir = tempfile::tempdir().unwrap();
    let out = run(&["--target", "v9.9.9", dir.path().to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error: "));
}

/// Known answers on PX4 itself; set FORK_PREFLIGHT_PX4 to a clone with the release tags.
fn px4_clone_at(rev: &str) -> Option<tempfile::TempDir> {
    let px4 = std::env::var("FORK_PREFLIGHT_PX4").ok()?;
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec![
            "clone",
            "-q",
            "--shared",
            "--no-checkout",
            px4.as_str(),
            dir.path().to_str().unwrap(),
        ],
        vec!["-C", dir.path().to_str().unwrap(), "checkout", "-q", rev],
    ] {
        assert!(Command::new("git").args(&args).status().unwrap().success());
    }
    Some(dir)
}

#[test]
fn px4_main_assigns_eight_undefined_symbols() {
    let Some(dir) = px4_clone_at("95a0c23482329183d932ac99da00faa7e8d63143") else {
        eprintln!("skipped: FORK_PREFLIGHT_PX4 not set");
        return;
    };
    let out = run(&[
        "--target",
        "main",
        "--all-files",
        "--json",
        dir.path().to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let findings = report["findings"].as_array().unwrap();
    let symbols: std::collections::BTreeSet<&str> = findings
        .iter()
        .map(|f| f["symbol"].as_str().unwrap())
        .collect();
    let files: std::collections::BTreeSet<&str> = findings
        .iter()
        .map(|f| f["file"].as_str().unwrap())
        .collect();
    assert_eq!(findings.len(), 19);
    assert_eq!(files.len(), 11);
    assert_eq!(
        symbols.into_iter().collect::<Vec<_>>(),
        [
            "BOARD_MODEL",
            "BOARD_NAME",
            "BOARD_UAVCAN_TIMER_OVERRIDE",
            "BOARD_VENDOR",
            "MODULES_DIFFERENTIAL_DRIVE",
            "MODULES_FW_POS_CONTROL",
            "MODULES_ROVER_POS_CONTROL",
            "UXRCE_DDS_CLIENT_SERIAL"
        ]
    );
    assert_eq!(report["files_checked"], 322);
    assert!(report["notice"]
        .as_str()
        .unwrap()
        .contains("without warranty"));
}

#[test]
fn px4_main_after_the_cleanup_has_none() {
    // PX4 pull request 29670: the same tree without the 19 stale lines.
    let Some(dir) = px4_clone_at("3e0e89e484") else {
        eprintln!("skipped: FORK_PREFLIGHT_PX4 not set");
        return;
    };
    let out = run(&[
        "--target",
        "main",
        "--all-files",
        dir.path().to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
}

#[test]
fn px4_v1_17_to_v1_18_rc1() {
    let Some(dir) = px4_clone_at("v1.17.0") else {
        eprintln!("skipped: FORK_PREFLIGHT_PX4 not set");
        return;
    };
    let out = run(&[
        "--target",
        "v1.18.0-rc1",
        "--all-files",
        "--json",
        dir.path().to_str().unwrap(),
    ]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["base"], "v1.17.0");
    let removed: Vec<&str> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["defined_at_base"] == true)
        .map(|f| f["symbol"].as_str().unwrap())
        .collect();
    // Only the symbol v1.18.0-rc1 removed that boards assign; COMMON_GPIO is not assigned by any board.
    assert_eq!(removed, ["DRIVERS_POWER_MONITOR_PM_SELECTOR_AUTERION"; 9]);
}
