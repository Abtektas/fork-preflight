#!/usr/bin/env bash
# SPDX-License-Identifier: BSD-3-Clause
# Runs fork-preflight for action.yml: the text report into the log, SARIF into a file.
# Inputs come from the environment (FP_*), never from expressions in the script.
set -euo pipefail

output() {
	if [ -n "${GITHUB_OUTPUT:-}" ]; then
		echo "$1=$2" >>"$GITHUB_OUTPUT"
	fi
}

bool() {
	case "$2" in
	true | false) ;;
	*)
		echo "::error::$1 must be true or false, not '$2'"
		output exit-code 2
		exit 2
		;;
	esac
}

bool all-files "${FP_ALL_FILES:-false}"
bool fail-on-findings "${FP_FAIL_ON_FINDINGS:-true}"
if [ -z "${FP_TARGET:-}" ]; then
	echo "::error::the target input is required"
	output exit-code 2
	exit 2
fi

if [ "$(git -C "${FP_PATH:-.}" rev-parse --is-shallow-repository)" = true ] && [ "$FP_ALL_FILES" != true ]; then
	echo "::error::the repository is a shallow clone; check it out with fetch-depth: 0 so that the base release is in its history"
	output exit-code 2
	exit 2
fi

args=(--target "$FP_TARGET")
if [ -n "${FP_BASE:-}" ]; then
	args+=(--base "$FP_BASE")
fi
if [ "$FP_ALL_FILES" = true ]; then
	args+=(--all-files)
fi

status=0
"$FP_BIN" "${args[@]}" "${FP_PATH:-.}" || status=$?
if [ "$status" -ge 2 ]; then
	echo "::error::fork-preflight stopped with an error (exit status $status)"
	output exit-code "$status"
	exit "$status"
fi

sarif_status=0
"$FP_BIN" "${args[@]}" --sarif "${FP_PATH:-.}" >"$FP_SARIF_FILE" || sarif_status=$?
if [ "$sarif_status" -ne "$status" ]; then
	echo "::error::the SARIF run ended with exit status $sarif_status, the text run with $status"
	output exit-code 2
	exit 2
fi
output exit-code "$status"
output sarif-file "$FP_SARIF_FILE"

if [ "$status" -eq 1 ] && [ "$FP_FAIL_ON_FINDINGS" = true ]; then
	echo "::error::fork-preflight reported findings (see the report above and $FP_SARIF_FILE)"
	exit 1
fi
