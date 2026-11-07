# fork-preflight

A pre-upgrade check for forks of the PX4 Autopilot. Before a fork merges a
new PX4 release, it will list the lines in the fork's own files that name
something the new release no longer defines:

- Kconfig symbols in `*.px4board` files (PX4 ignores an unknown symbol
  silently),
- parameters in startup and airframe scripts (`param set-default` with an
  unknown name prints an error at boot and the script goes on),
- module commands in startup scripts whose module was removed.

**Status: early development, not released.** The release data (milestone P1)
and the first scanner check, `kconfig-undefined` (milestone P2), exist. The
parameter and command checks do not exist yet.

## Scanner

```sh
cargo build --release
target/release/fork-preflight --target v1.18.0-rc1 path/to/fork
```

- `--target` is the release the fork will merge (`--list-releases` lists the
  releases in the data, which is built into the binary).
- The base is the newest release in the data whose commit is in HEAD's
  history; `--base` names it instead.
- It reads the committed state of HEAD. In files the fork added since the
  base it checks every line; in upstream files the fork modified it checks
  only the lines the fork added or changed, because the merge replaces the
  rest. `--all-files` checks every board file in HEAD.
- `kconfig-undefined`: a `CONFIG_X=...` or `# CONFIG_X is not set` line in a
  `boards/<vendor>/<model>/<label>.px4board` file whose symbol the target
  release does not define (globally or in that board's own Kconfig), unless
  a Kconfig file the fork added or changed defines it. The finding says
  whether the base release defined the symbol.
- Output is text, or `--json`. Exit status 0 without findings, 1 with
  findings, 2 on a usage or data error. Every report ends with the notices of
  [DISCLAIMER.md](DISCLAIMER.md).

On PX4's own main at 95a0c23482, `--target main --all-files` reports 19 lines
in 11 board files that assign 8 undefined symbols (the lines PX4 pull
request 29670 removes); on that pull request's commit it reports none.

See [DISCLAIMER.md](DISCLAIMER.md): no warranty, no liability, not a safety
tool. This project is not affiliated with or endorsed by the PX4 project or
the Dronecode Foundation. PX4 is a trademark of Dronecode Project, Inc.

## Release data

`data/releases/<release>.json` says what one PX4 release defines. The
releases and their pinned commits are in `generator/releases.json`: every
final release from v1.14.0 to v1.17.0, v1.18.0-rc1, and `main` at one pinned
commit. The layout is `data/schema/release.schema.json`.

The facts come from the release's own source tree and tools, not from text
searches:

- **Kconfig symbols.** kconfiglib loads the tree's root `Kconfig` with the
  variables PX4's `cmake/kconfig.cmake` sets. Files the build generates first
  (the Zenoh topic catalog on `main`) are generated with the tree's own
  script. A board's own `Kconfig`, where the root `Kconfig` sources one, adds
  `board_symbols`. An unknown variable in a Kconfig file stops the generator.
- **Module commands.** The `MAIN` argument of every `px4_add_module()` call.
- **Parameters.** Names from the
  [param-lineage](https://github.com/Abtektas/param-lineage) snapshot of the
  release, and the parameters the release removed with their successors, at
  the commit pinned in `generator/releases.json`. param-lineage has no
  snapshot of `main`, so `main` has no parameter data (`parameters` is null
  and `notes` says so).

### Regenerate

Needs a clone of PX4-Autopilot with the release tags, a clone of
param-lineage at the pinned commit, Python 3.12 and the pinned packages:

```sh
python3.12 -m venv .venv
.venv/bin/pip install --require-hashes -r generator/requirements.txt
.venv/bin/python -I generator/generate.py --px4 ../PX4-Autopilot --param-lineage ../param-lineage
```

`--check` regenerates every file and compares it byte by byte with
`data/releases` without writing (exit 1 on a difference). `--release R`
limits the run.

### Tests

```sh
.venv/bin/pip install --require-hashes -r generator/requirements-test.txt
.venv/bin/python -I -m unittest discover -s tests
```

Set `FORK_PREFLIGHT_PX4` and `FORK_PREFLIGHT_PARAM_LINEAGE` to the two clones
to also run the regeneration check from the tests.

The scanner (Rust 1.94.0, from `rust-toolchain.toml`):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
FORK_PREFLIGHT_PX4=../PX4-Autopilot cargo test
```

Without `FORK_PREFLIGHT_PX4` the known-answer tests on PX4 are skipped.

## License

BSD-3-Clause, see [LICENSE](LICENSE). The data is derived from PX4 source
code, see [LICENSE-THIRD-PARTY](LICENSE-THIRD-PARTY).
