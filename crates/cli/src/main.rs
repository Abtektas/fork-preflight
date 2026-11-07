// SPDX-License-Identifier: BSD-3-Clause
//! `fork-preflight`: report the lines of a PX4 fork that name what the target release no longer defines.

use clap::Parser;
use fork_preflight_core::{check, data, git::Repo, legal, sarif};
use std::path::PathBuf;
use std::process::ExitCode;

const EXIT_FINDINGS: u8 = 1;
const EXIT_ERROR: u8 = 2;

#[derive(Parser)]
#[command(
    name = "fork-preflight",
    version,
    disable_version_flag = true,
    about = "Report the lines in a PX4 fork's own files that name a Kconfig symbol, parameter or command the target release does not define.",
    after_help = "Exit status: 0 no findings, 1 findings, 2 usage or data error.\n\
                  Reads the committed state of HEAD, not uncommitted changes."
)]
struct Cli {
    /// Release the fork will merge (see --list-releases)
    #[arg(long, required_unless_present_any = ["list_releases", "license", "version"])]
    target: Option<String>,
    /// Release the fork is based on [default: newest data release that HEAD contains]
    #[arg(long)]
    base: Option<String>,
    /// Where the fork left upstream: that commit, or the upstream branch (the change set starts at
    /// the merge-base of HEAD and REV) [default: the base release, else the merge-base of HEAD
    /// and the data's main commit]
    #[arg(long, value_name = "REV")]
    fork_point: Option<String>,
    /// Check every board file and script in HEAD, not only the lines the fork added or changed since the base
    #[arg(long)]
    all_files: bool,
    /// Print the report as JSON
    #[arg(long, conflicts_with = "sarif")]
    json: bool,
    /// Print the report as SARIF 2.1.0 (for GitHub code scanning)
    #[arg(long)]
    sarif: bool,
    /// List the releases the embedded data covers
    #[arg(long)]
    list_releases: bool,
    /// Print the license texts
    #[arg(long)]
    license: bool,
    /// Print the version and the notices
    #[arg(short = 'V', long)]
    version: bool,
    /// Repository to check
    #[arg(default_value = ".")]
    repo: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn run(cli: Cli) -> Result<u8, String> {
    if cli.version {
        println!(
            "fork-preflight {}\n\n{}",
            env!("CARGO_PKG_VERSION"),
            legal::notice()
        );
        return Ok(0);
    }
    if cli.license {
        println!("{}\n\n{}", legal::LICENSE, legal::LICENSE_THIRD_PARTY);
        return Ok(0);
    }
    let releases = data::releases()?;
    if cli.list_releases {
        for release in releases.values() {
            println!("{} {}", release.release, release.source.commit);
        }
        return Ok(0);
    }
    let repo = Repo::open(&cli.repo)?;
    let options = check::Options {
        base: cli.base,
        target: cli.target.expect("clap requires --target"),
        all_files: cli.all_files,
        fork_point: cli.fork_point,
    };
    let report = check::run(&repo, &releases, &options)?;
    if cli.json {
        let mut value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
        value["tool"] = "fork-preflight".into();
        value["version"] = env!("CARGO_PKG_VERSION").into();
        value["notice"] = legal::notice().into();
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        );
    } else if cli.sarif {
        let log = sarif::to_sarif(
            &report,
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_REPOSITORY"),
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&log).map_err(|e| e.to_string())?
        );
    } else {
        print_text(&report);
    }
    Ok(if report.findings.is_empty() {
        0
    } else {
        EXIT_FINDINGS
    })
}

fn print_text(report: &check::Report) {
    for finding in &report.findings {
        println!(
            "{}:{}: {}: {}",
            finding.file, finding.line, finding.rule, finding.message
        );
        println!("    {}", finding.text.trim_start());
        if let Some(hint) = &finding.hint {
            println!("    hint: {hint}");
        }
    }
    let base = report.base.as_deref().unwrap_or("none");
    if !report.skipped.is_empty() {
        println!("\nSkipped:");
        for skipped in &report.skipped {
            println!("- {skipped}");
        }
    }
    println!(
        "\n{} finding(s) in {} file(s), {} from base {base} to target {}, HEAD {}.",
        report.findings.len(),
        report.files_checked,
        report.scope,
        report.target,
        &report.head[..report.head.len().min(10)],
    );
    if let Some(point) = &report.fork_point {
        println!(
            "Changes since {} ({}).",
            &point.commit[..point.commit.len().min(10)],
            point.from
        );
    }
    println!("\n{}", legal::notice());
}
