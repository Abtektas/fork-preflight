# Changelog

## Unreleased

- Release data generator (`generator/generate.py`) and data for every final
  PX4 release from v1.14.0 to v1.17.0, v1.18.0-rc1 and main at 95a0c23482:
  Kconfig symbols (generated Zenoh catalog and board Kconfig included),
  module commands, and parameters from param-lineage. JSON Schema in
  `data/schema/`. `--check` regenerates the data byte for byte.
- Scanner `fork-preflight` (Rust) with the `kconfig-undefined` check: board
  file lines whose Kconfig symbol the target release does not define, limited
  to what the fork added or changed since its base release (`--all-files`
  for every board file). Text and JSON output; the release data is built in.
- `param-removed`: script lines that name a parameter the target release does
  not have, with param-lineage's removal kind and successors.
- `command-removed`: script commands that the base release declares and the
  target does not, with the declaring `CMakeLists.txt`.
- Fork definitions in Kconfig, `CMakeLists.txt`, C/C++ and YAML module
  configs count as defined. Rules without data are reported as skipped.
- `--sarif` (SARIF 2.1.0) and a composite GitHub Action (`action.yml`).
- `--fork-point REV` (Action input `fork-point`): the change set starts at the
  merge-base of HEAD and REV. A fork of PX4's main branch, where no release
  is an ancestor of HEAD, starts at the merge-base with the data's `main`
  commit instead of stopping with an error. Reports name the fork point.
- A fork point on `main` that the target does not contain stops with an
  error instead of reporting every name `main` added after the target's
  release branch was cut.
- A finding whose name only upstream lines of a fork-modified file define
  gets a hint naming that file (found on a fork that extends a module the
  target release removes).
- Data format 2: `kconfig.removed` records the Kconfig symbols each release
  removed, the PX4 commits that removed them and the successors that PX4's
  git history shows (renamed, set by in-tree boards instead, or selected).
  `kconfig-undefined` findings for a symbol the base defined carry this as a
  hint, following successors to the target. The generator needs a full PX4
  clone and records the git version.
