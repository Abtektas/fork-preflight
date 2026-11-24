# Changelog

## 0.1.0 (not released yet)

First release, with data for PX4 v1.18.0. The data ends at v1.18.0 and main
at 95a0c23482.

### Release data

- Generator (`generator/generate.py`, Python 3.12, pinned packages with
  hashes) and data for every final PX4 release from v1.14.0 to v1.18.0,
  v1.18.0-rc1 and main at one pinned commit. JSON Schema in `data/schema/`.
  `--check` regenerates the data byte for byte.
- Kconfig symbols from kconfiglib on the release's own tree, with the
  generated Zenoh catalog and board Kconfig files.
- Module commands (`MAIN` of `px4_add_module`) and the POSIX shell builtins
  that `platforms/common/apps.cpp.in` registers (`sleep`, `list_tasks`,
  `list_files`, `shutdown`), so a module command that becomes a builtin is
  not reported as removed.
- Parameters and their removals from param-lineage 0.1.0 (be8fd9cc8e). It
  keeps only final releases, so v1.18.0-rc1 and main have none.
- Data format 2: `kconfig.removed` records the Kconfig symbols each release
  removed, the PX4 commits that removed them and the successors that PX4's
  git history shows (renamed, set by in-tree boards instead, selected, or
  named by the removing commit's message).
  The generator needs a full PX4 clone and records the git version.

### Scanner

- `fork-preflight` (Rust 1.94.0) with the release data built in. Text, JSON
  and SARIF 2.1.0 output; exit status 0, 1 (findings) or 2 (error).
- `kconfig-undefined`: board file lines whose Kconfig symbol the target
  release does not define. Findings for a symbol the base defined carry the
  removing commits and successors as a hint, followed to the target.
- `param-removed`: script lines that name a parameter the target release
  does not have, with param-lineage's removal kind and successors.
- `command-removed`: script commands that the base release declares and the
  target does not, with the declaring `CMakeLists.txt`.
- Only lines the fork added or changed since its fork point are checked;
  `--all-files` checks every board file and script.
- The fork point is the base release, `--fork-point REV` (merge-base of HEAD
  and REV), or for a fork of PX4's main branch the merge-base with the data's
  `main` commit. Reports name it. A fork point on `main` that the target does
  not contain stops with an error.
- Fork definitions in Kconfig, `CMakeLists.txt`, C/C++ and YAML module
  configs count as defined. A name that only upstream lines of a
  fork-modified file define gets a hint naming that file.
- Without a release base, `kconfig-undefined` reads the fork point's own
  Kconfig files with git to say whether the symbol was already undefined
  there or removed in between. Generated Zenoh topic symbols stay unknown.
- Rules without data are reported as skipped.
- `--version` and every report carry the notices of `DISCLAIMER.md`;
  `--license` prints the license texts.

### GitHub Action

- Composite action (`action.yml`) that builds the scanner with the pinned
  toolchain, writes the text report to the log and SARIF to a file. Run
  locally in a `rust:1.94.0` linux/amd64 container, not yet on GitHub.
