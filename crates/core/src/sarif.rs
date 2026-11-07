// SPDX-License-Identifier: BSD-3-Clause
//! The report as SARIF 2.1.0, for GitHub code scanning and other SARIF viewers.

use crate::check::{Report, RULES};
use crate::legal;
use serde_json::{json, Value};

pub const SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";

/// One SARIF log with one run. Paths are relative to the repository root (`%SRCROOT%`).
pub fn to_sarif(report: &Report, tool_version: &str, information_uri: &str) -> Value {
    let rules: Vec<Value> = RULES
        .iter()
        .map(|(id, description)| {
            json!({
                "id": id,
                "name": id,
                "shortDescription": {"text": description},
                "defaultConfiguration": {"level": "warning"},
            })
        })
        .collect();
    let results: Vec<Value> = report
        .findings
        .iter()
        .map(|finding| {
            let rule_index = RULES
                .iter()
                .position(|(id, _)| *id == finding.rule)
                .expect("every finding names a known rule");
            let mut text = finding.message.clone();
            if let Some(hint) = &finding.hint {
                text.push_str(&format!(". {hint}"));
            }
            json!({
                "ruleId": finding.rule,
                "ruleIndex": rule_index,
                "level": "warning",
                "message": {"text": text},
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {"uri": finding.file, "uriBaseId": "%SRCROOT%"},
                        "region": {"startLine": finding.line, "snippet": {"text": finding.text}},
                    }
                }],
                "properties": {
                    "symbol": finding.symbol,
                    "definedAtBase": finding.defined_at_base,
                },
            })
        })
        .collect();
    json!({
        "$schema": SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "fork-preflight",
                "version": tool_version,
                "informationUri": information_uri,
                "rules": rules,
            }},
            "automationDetails": {"id": format!("fork-preflight/{}/", report.target)},
            "originalUriBaseIds": {"%SRCROOT%": {"description": {"text": "The repository root."}}},
            "invocations": [{
                "executionSuccessful": true,
                "toolExecutionNotifications": report.skipped.iter().map(|s| json!({
                    "level": "note",
                    "message": {"text": format!("skipped {s}")},
                })).collect::<Vec<_>>(),
            }],
            "results": results,
            "properties": {
                "base": report.base,
                "target": report.target,
                "head": report.head,
                "scope": report.scope,
                "filesChecked": report.files_checked,
                "notice": legal::notice(),
            },
        }],
    })
}
