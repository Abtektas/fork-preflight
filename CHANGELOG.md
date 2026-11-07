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
