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
    assert!(findings.iter().all(|f| f["rule"] == "kconfig-undefined"));
    // 322 board files and 502 scripts; no base, and main has no parameter data.
    assert_eq!(report["files_checked"], 824);
    assert_eq!(
        report["skipped"],
        serde_json::json!([
            "param-removed: the data has no parameters for main",
            "command-removed: needs a base release"
        ])
    );
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
fn px4_v1_17_to_v1_18() {
    let Some(dir) = px4_clone_at("v1.17.0") else {
        eprintln!("skipped: FORK_PREFLIGHT_PX4 not set");
        return;
    };
    let out = run(&[
        "--target",
        "v1.18.0",
        "--all-files",
        "--json",
        dir.path().to_str().unwrap(),
    ]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["base"], "v1.17.0");
    let removed = removed_at(&report);
    // Only the symbol v1.18.0-rc1 removed that boards assign; COMMON_GPIO is not assigned by any board.
    assert_eq!(
        removed["kconfig-undefined"],
        [("DRIVERS_POWER_MONITOR_PM_SELECTOR_AUTERION".to_string(), 9)].into()
    );
    // The 11 removed parameters that v1.17.0's own scripts still set (commented lines do not count).
    let params: std::collections::BTreeMap<String, usize> = [
        ("COM_ARM_SDCARD", 3),
        ("COM_OBC_LOSS_T", 1),
        ("EKF2_GPS_DELAY", 7),
        ("EKF2_GPS_POS_X", 4),
        ("EKF2_GPS_POS_Z", 3),
        ("FD_ACT_MOT_TOUT", 3),
        ("FD_ESCS_EN", 4),
        ("MNT_RANGE_PITCH", 1),
        ("SENS_IMU_TEMP", 4),
        ("SENS_TEMP_ID", 11),
        ("SIM_BAT_ENABLE", 1),
    ]
    .into_iter()
    .map(|(n, c)| (n.to_string(), c))
    .collect();
    assert_eq!(removed["param-removed"], params);
    assert_eq!(
        removed["command-removed"],
        [
            ("core_heater".to_string(), 1),
            ("pm_selector_auterion".to_string(), 2)
        ]
        .into()
    );
    let finding = |symbol: &str| {
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["symbol"] == symbol)
            .unwrap()
            .clone()
    };
    assert_eq!(
        finding("SENS_TEMP_ID")["hint"],
        "param-lineage: removed in v1.18.0 (migrated, PX4 converts a stored value); successors: HEATER1_SENS_ID"
    );
    assert_eq!(
        finding("core_heater")["hint"],
        "v1.17.0 declares it in boards/cuav/x25-evo/core_heater/CMakeLists.txt"
    );
    assert_eq!(
        finding("DRIVERS_POWER_MONITOR_PM_SELECTOR_AUTERION")["hint"],
        "removed in v1.18.0-rc1 by 4ac7853479 (auterion autostarter: Added autostart for PWM_EXPANDER, generalized c\u{2026} (#25925))"
    );
    // PWM_MAIN_FUNC9 to 11 (SITL has 16 outputs, the data 8) are numbered instances, not findings.
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| !f["symbol"].as_str().unwrap().starts_with("PWM_MAIN_FUNC")));
}

#[test]
fn px4_v1_18_rc1_base_has_no_parameter_data() {
    // param-lineage keeps final releases only, so a fork of v1.18.0-rc1 has no base parameters:
    // the findings stay, and the message says that the base is not known.
    let Some(dir) = px4_clone_at("v1.18.0-rc1") else {
        eprintln!("skipped: FORK_PREFLIGHT_PX4 not set");
        return;
    };
    let out = run(&[
        "--target",
        "v1.18.0",
        "--all-files",
        "--json",
        dir.path().to_str().unwrap(),
    ]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["base"], "v1.18.0-rc1");
    let findings = report["findings"].as_array().unwrap();
    let params: Vec<&serde_json::Value> = findings
        .iter()
        .filter(|f| f["rule"] == "param-removed")
        .collect();
    assert_eq!(params.len(), 52);
    assert!(params.iter().all(|f| f["defined_at_base"].is_null()));
    let f = params
        .iter()
        .find(|f| f["symbol"] == "FD_ACT_MOT_TOUT")
        .unwrap();
    assert_eq!(
        f["message"],
        "FD_ACT_MOT_TOUT is not a parameter in v1.18.0; the data has no parameters for v1.18.0-rc1, so it may already be missing there"
    );
}

#[test]
fn px4_v1_16_2_to_main_kconfig_hints() {
    let Some(dir) = px4_clone_at("v1.16.2") else {
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
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let hint = |symbol: &str| {
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["symbol"] == symbol && f["rule"] == "kconfig-undefined")
            .unwrap()["hint"]
            .clone()
    };
    assert_eq!(
        hint("MODULES_FW_POS_CONTROL"),
        "removed in v1.17.0 by 8c1f7ec7c0 (rename FWPositionController to FWModeManager); in-tree boards set MODULES_FW_LATERAL_LONGITUDINAL_CONTROL (61/64) and MODULES_FW_MODE_MANAGER (79/79) instead"
    );
    assert_eq!(
        hint("NUM_MISSION_ITMES_SUPPORTED"),
        "removed in main by 1476321315 (fix(dataman): correct mission items Kconfig name (#28478)); renamed to NUM_MISSION_ITEMS_SUPPORTED"
    );
    assert_eq!(
        hint("MODULES_ROVER_POS_CONTROL"),
        "removed in v1.17.0 by e266d1f22f (rover: remove deprecated module)"
    );
    let removed = removed_at(&report);
    let symbols: Vec<&String> = removed["kconfig-undefined"].keys().collect();
    assert_eq!(
        symbols,
        [
            "BOARD_UAVCAN_PERIPHERALS",
            "BOARD_UAVCAN_TIMER_OVERRIDE",
            "DRIVERS_POWER_MONITOR_PM_SELECTOR_AUTERION",
            "MODULES_FW_POS_CONTROL",
            "MODULES_ROVER_POS_CONTROL",
            "NUM_MISSION_ITMES_SUPPORTED"
        ]
    );
}

/// Count of findings per rule and symbol whose symbol the base release defines.
fn removed_at(
    report: &serde_json::Value,
) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, usize>> {
    let mut map: std::collections::BTreeMap<String, std::collections::BTreeMap<String, usize>> =
        ["kconfig-undefined", "param-removed", "command-removed"]
            .into_iter()
            .map(|r| (r.to_string(), Default::default()))
            .collect();
    for f in report["findings"].as_array().unwrap() {
        if f["defined_at_base"] == true {
            *map.get_mut(f["rule"].as_str().unwrap())
                .unwrap()
                .entry(f["symbol"].as_str().unwrap().to_string())
                .or_default() += 1;
        }
    }
    map
}

#[test]
fn px4_v1_18_and_main_still_set_parameters_v1_18_removed() {
    // Lines in PX4's own scripts at v1.18.0 and main (95a0c23482) that set parameters
    // v1.18.0 removed. No script there runs a command that v1.17.0 has and they lack.
    for (rev, target) in [
        ("v1.18.0", "v1.18.0"),
        ("95a0c23482329183d932ac99da00faa7e8d63143", "main"),
    ] {
        let Some(dir) = px4_clone_at(rev) else {
            eprintln!("skipped: FORK_PREFLIGHT_PX4 not set");
            return;
        };
        let out = run(&[
            "--base",
            "v1.17.0",
            "--target",
            target,
            "--all-files",
            "--json",
            dir.path().to_str().unwrap(),
        ]);
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let removed = removed_at(&report);
        assert!(removed["command-removed"].is_empty(), "{rev}");
        if target == "main" {
            assert!(report["findings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f["rule"] == "kconfig-undefined"));
            continue;
        }
        assert_eq!(
            removed["param-removed"],
            [
                ("COM_ARM_SDCARD".to_string(), 1),
                ("FD_ACT_MOT_TOUT".to_string(), 3),
                ("MPC_LAND_RC_HELP".to_string(), 2)
            ]
            .into(),
            "{rev}"
        );
    }
}

#[test]
fn sarif_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .arg("-C")
            .arg(path)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.invalid",
                "-c",
                "commit.gpgsign=false"
            ])
            .args(args)
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q"]);
    let board = path.join("boards/acme/x1");
    std::fs::create_dir_all(&board).unwrap();
    std::fs::write(board.join("default.px4board"), "CONFIG_NOT_A_SYMBOL=y\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "fork"]);
    let out = run(&[
        "--target",
        "main",
        "--all-files",
        "--sarif",
        path.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let log: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(log["version"], "2.1.0");
    let run = &log["runs"][0];
    assert_eq!(run["tool"]["driver"]["name"], "fork-preflight");
    let rules: Vec<&str> = run["tool"]["driver"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        rules,
        ["kconfig-undefined", "param-removed", "command-removed"]
    );
    let results = run["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["ruleId"], "kconfig-undefined");
    assert_eq!(results[0]["ruleIndex"], 0);
    let location = &results[0]["locations"][0]["physicalLocation"];
    assert_eq!(
        location["artifactLocation"]["uri"],
        "boards/acme/x1/default.px4board"
    );
    assert_eq!(location["region"]["startLine"], 1);
    assert!(run["properties"]["notice"]
        .as_str()
        .unwrap()
        .contains("accept no liability"));
    assert_eq!(
        run["invocations"][0]["toolExecutionNotifications"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let both = run_args_conflict();
    assert_eq!(both, Some(2));
}

fn run_args_conflict() -> Option<i32> {
    run(&["--target", "main", "--json", "--sarif"])
        .status
        .code()
}
