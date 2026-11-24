# fork-preflight

A pre-upgrade check for forks of the PX4 Autopilot. Before a fork merges a
new PX4 release, it will list the lines in the fork's own files that name
something the new release no longer defines:

- Kconfig symbols in `*.px4board` files (PX4 ignores an unknown symbol
  silently),
- parameters in startup and airframe scripts (`param set-default` with an
  unknown name prints an error at boot and the script goes on),
- module commands in startup scripts whose module was removed.

**Status: 0.1.0, the first release, with data up to PX4 v1.18.0.** The
release data, the three checks, text/JSON/SARIF output and a GitHub Action
have been run on PX4's own tree and, locally, on six public forks, whose
findings were reviewed for false positives. [CHANGELOG.md](CHANGELOG.md)
lists what it contains.

## Scanner

```sh
cargo build --release --locked
target/release/fork-preflight --target v1.18.0 path/to/fork
```

`cargo install --locked --path crates/cli` puts the same binary in
`~/.cargo/bin`. `--help` lists the options, `--version` prints the version
with the notices below, and `--license` prints the license texts.

- `--target` is the release the fork will merge (`--list-releases` lists the
  releases in the data, which is built into the binary, with their PX4
  commits).
- The base is the newest release in the data whose commit is in HEAD's
  history; `--base` names it instead.
- The change set starts at the fork point: the base release's commit, or
  `--fork-point REV` (the merge-base of HEAD and REV, so REV can be the commit
  where the fork left upstream or the upstream branch itself). A fork of
  PX4's `main` branch contains no release commit; then the fork point is the
  merge-base of HEAD and the data's `main` commit, which the repository must
  have (`git fetch https://github.com/PX4/PX4-Autopilot.git main`). Without a
  release base, `command-removed` is skipped, `kconfig-undefined` asks the
  fork point's tree whether it defined the symbol (see below), and "defined
  at base" is unknown for parameters. The report names the fork point and
  how it was chosen.
- A fork point on `main` that the target does not contain (the fork left
  `main` after the target's release branch was cut) stops with an error: the
  fork already has upstream work the target lacks, so it is not an upgrade.
  Choose a newer target, such as `main`. The check needs the data's `main`
  and target commits in the repository and is skipped without them.
- It reads the committed state of HEAD. In files the fork added since the
  base it checks every line; in upstream files the fork modified it checks
  only the lines the fork added or changed, because the merge replaces the
  rest. `--all-files` checks every board file and script in HEAD.
- Output is text, `--json` or `--sarif` (SARIF 2.1.0). Exit status 0 without
  findings, 1 with findings, 2 on a usage or data error. Every report ends
  with the notices of [DISCLAIMER.md](DISCLAIMER.md). A rule whose data is
  missing is listed as skipped, not passed.

### Checks

| Rule | Lines | Reported when |
|---|---|---|
| `kconfig-undefined` | `CONFIG_X=...` and `# CONFIG_X is not set` in `boards/<vendor>/<model>/<label>.px4board` | the target release does not define the symbol, globally or in that board's own Kconfig |
| `param-removed` | the parameter of `param set`, `set-default`, `compare`, `greater`, `bitset`, `bitclear`, `find`, and the names of `touch` and `reset`, in scripts | the target release has no such parameter |
| `command-removed` | the command word of each command in scripts | the base release declares the command (`MAIN` of `px4_add_module`, or a POSIX shell builtin in `platforms/common/apps.cpp.in`) and the target does not |

Scripts are the files in `ROMFS/<dir>/init.d*/`, `boards/<vendor>/<model>/init/`
and `target/`, `platforms/nuttx/init/` and `posix-configs/`, without
`CMakeLists.txt`, templates (`.in`, `.jinja`) and documents. Comments,
here-documents and quoted text are not read; a name with `$` is not checked.

Each finding says whether the base release defined the name ("removed or
renamed in between") or not (a typo, or older rot). Without a release base,
a `kconfig-undefined` finding says the same about the fork point ("the fork
point <commit> defines it", "nor at the fork point <commit>"). The data has no
tree there, so the scanner reads the fork point's own `Kconfig*` files with
git and takes their `config`/`menuconfig` lines, a file under
`boards/<vendor>/<model>/` counting for that board only and `nuttx-config/`
files (NuttX options) left out. On every data release this text search
gives the data's kconfiglib set, except one file no Kconfig sources
(`DRIVERS_GHST_RC` on two modalai boards); a Zenoh topic symbol
(`ZENOH_PUBSUB_*`) of a tree that generates them at build time stays
unknown. A `param-removed` finding
carries what [param-lineage](https://github.com/Abtektas/param-lineage)
records about the removal: its kind (`migrated`: PX4 converts a stored
value; `renamed`: a successor exists but PX4 does not convert the value;
`replaced`, `fixed`, `removed`) and the successors. A `command-removed`
finding names the `CMakeLists.txt` that declared the command at the base and
the commands that file declares at the target. A `kconfig-undefined` finding
for a symbol the base defined names the release and the PX4 commits that
removed it, and what PX4's git history shows took its place: the name the
same commit added (`renamed to ...`), the symbols in-tree boards set instead
(`in-tree boards set ... (79/79) instead`, boards that changed over out of
boards that set it), the symbols it selected, or the name the removing
commit's message gives (`its commit message names ...`). A successor that a later
release removed again is followed to the target ("then in main ..."), and
the hint says when the target defines none of them. These are records of
what upstream did, not advice; read the named commits before changing a
board file.

Definitions the fork adds or changes count as defined: `config`/`menuconfig`
in Kconfig files, `MAIN` in `CMakeLists.txt`, `PARAM_DEFINE_INT32/FLOAT` in
C/C++ files, and the names under `definitions:` (with `${i}` instances) and
`param_prefix:` in YAML module configs. A name that only upstream lines of a
file the fork modified define is still reported, with a hint naming that
file: the merge brings upstream's version of those lines (a removed module
means a modify/delete conflict), so the name stays defined only if the fork
keeps the definition.

What the data does not cover:

- **Parameters of `main` and release candidates.** param-lineage has no
  snapshot of `main` and keeps only final releases, so `param-removed` is
  skipped for `--target main` and `--target v1.18.0-rc1`.
- **Board-dependent parameters.** The parameter data is PX4's own parameter
  metadata. It has one count of output channels (`PWM_MAIN_FUNC1` to `8`),
  so a numbered parameter above the count (`PWM_MAIN_FUNC9`) is not reported
  unless the base had it. Parameters that only a board directory defines
  (v1.18.0: `PWM_VOLT_SEL`, `PWM_LEVEL_CONT`, `SLNK_RADIO_*`) are not in
  it; a script that sets one is reported as "not in the parameter data".
- **Kconfig successors the rules miss.** The successors come from fixed
  rules over the git history between two data releases (measured on all 53
  symbols the data releases removed: 21 found, 0 wrong, 6 missed). A
  successor that already existed before the removal, a rename on one board
  only, or a split into several new symbols is not found; the hint then
  names only the removing commits. Board-only symbols get no removal record.
- **Kconfig at a fork point.** The text search does not follow `source`
  lines, so a `Kconfig*` file that nothing sources still counts, and
  definitions generated at build time other than the Zenoh topics are not
  seen. Parameters at a fork point are not known.
- **Builtins.** Shell builtins and commands that are not PX4 modules
  (`echo`, `set`, `list_files`) are never reported.
- **History that hides upstream work.** The change set comes from git
  ancestry. When a fork squashes or copies newer upstream files into its own
  commits, those files count as the fork's, and their stale lines are
  reported too. `--fork-point` with a later upstream commit narrows it.

### Known answers on PX4 itself

| Tree | Command | Result |
|---|---|---|
| main 95a0c23482 | `--target main --all-files` | 19 lines in 11 board files assign 8 undefined Kconfig symbols (what PX4 pull request 29670 removes); parameters and commands skipped |
| PX4 pull request 29670, 3e0e89e484 | `--target main --all-files` | no findings |
| v1.17.0 | `--target v1.18.0 --all-files` | 9 boards assign `DRIVERS_POWER_MONITOR_PM_SELECTOR_AUTERION`; 42 script lines set 11 parameters that v1.18.0 removed (`SENS_TEMP_ID` 11, `EKF2_GPS_DELAY` 7, ...); 3 lines start `core_heater` or `pm_selector_auterion` |
| v1.18.0 | `--base v1.17.0 --target v1.18.0 --all-files` | 6 lines of PX4's own scripts still set parameters that v1.18.0 removed (`FD_ACT_MOT_TOUT` 3, `MPC_LAND_RC_HELP` 2, `COM_ARM_SDCARD` 1); no removed command |

The parameter lines were checked against a plain text search of the same
trees: the same lines, apart from documentation pages and commented lines.

### GitHub Action

[action.yml](action.yml) builds the scanner with the pinned toolchain and runs
it on the checkout: the text report goes to the log, SARIF to a file. The
checkout needs the history back to the base release. A fork of PX4's `main`
branch also needs the data's `main` commit: add a step that runs
`git fetch https://github.com/PX4/PX4-Autopilot.git main`, or set `fork-point`.

```yaml
on: pull_request
permissions:
  contents: read
  security-events: write
jobs:
  preflight:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          fetch-depth: 0
      - uses: Abtektas/fork-preflight@<commit>
        with:
          target: v1.18.0
      - uses: github/codeql-action/upload-sarif@33d7b85da1a04171e211befe2c739303b9a4c066 # v4.39.3
        if: always()
        with:
          sarif_file: fork-preflight.sarif
```

Inputs: `target` (required), `base`, `fork-point`, `path`, `all-files`, `sarif-file`
(default `fork-preflight.sarif`), `fail-on-findings` (default `true`; errors
always fail). Outputs: `exit-code`, `sarif-file`. A shallow clone is refused
unless `all-files` is set. The action has been run locally, step by step,
in a `rust:1.94.0` linux/amd64 container; it has not run on GitHub yet.

See [DISCLAIMER.md](DISCLAIMER.md): no warranty, no liability, not a safety
tool. This project is not affiliated with or endorsed by the PX4 project or
the Dronecode Foundation. PX4 is a trademark of Dronecode Project, Inc.

## Release data

`data/releases/<release>.json` says what one PX4 release defines. The
releases and their pinned commits are in `generator/releases.json`: every
final release from v1.14.0 to v1.18.0, v1.18.0-rc1, and `main` at one pinned
commit. v1.18.0-rc1 and `main` have no parameter data: param-lineage keeps a
snapshot of final releases only, so `param-removed` is skipped when one of
them is the target, and for a fork based on one of them the message says that
the base has no parameter data (the parameter may already be missing there). The layout is `data/schema/release.schema.json`.

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
- **Removed Kconfig symbols.** `kconfig.removed` lists the global symbols the
  previous data release defines and this one does not, with up to three
  commits that deleted their `config` line (for a Zenoh topic, its msg file)
  and the successors the git history shows, from the first rule that
  matches: `renamed` (the deleting commit adds a `config` with a matching
  name or a batch prefix rename; a Zenoh msg file renamed by git), `boards`
  (in-tree board files that set it set the successor instead, in the same
  commits or between the two releases) or `selected` (it selected the
  successor). For a symbol these leave without a successor: `renamed` also
  when a sibling renamed in the same commit gives the name by the same token
  change, or when the commit that orphaned its Kconfig file added a matching
  name; `named` when the commit message says "rename A to B", "from A to B"
  or "equivalent to B". The rules are in `generator/kconfig_removed.py`. They read only
  git objects of the pinned commits, so the PX4 clone must have the full
  history (a shallow clone stops the generator), and the git version is
  recorded in `tools` because rename detection is git's. `--check` notes a
  different git version and still compares every value.

### Regenerate

Needs a full (not shallow) clone of PX4-Autopilot with the release tags, a clone of
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
FORK_PREFLIGHT_PX4="$PWD/../PX4-Autopilot" cargo test
```

`FORK_PREFLIGHT_PX4` is an absolute path to a PX4 clone with the release
tags; without it the known-answer tests on PX4 are skipped. The action's
script: `shellcheck action/run.sh`.

[RELEASING.md](RELEASING.md) says how to add a PX4 release to the data and
how to cut a release.

## License

BSD-3-Clause, see [LICENSE](LICENSE). The data is derived from PX4 source
code, see [LICENSE-THIRD-PARTY](LICENSE-THIRD-PARTY).
