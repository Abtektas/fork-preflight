# fork-preflight

A pre-upgrade check for forks of the PX4 Autopilot. Before a fork merges a
new PX4 release, it will list the lines in the fork's own files that name
something the new release no longer defines:

- Kconfig symbols in `*.px4board` files (PX4 ignores an unknown symbol
  silently),
- parameters in startup and airframe scripts (`param set-default` with an
  unknown name prints an error at boot and the script goes on),
- module commands in startup scripts whose module was removed.

**Status: early development, not released.** This repository has the release
data generator (milestone P1). The scanner does not exist yet.

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

## License

BSD-3-Clause, see [LICENSE](LICENSE). The data is derived from PX4 source
code, see [LICENSE-THIRD-PARTY](LICENSE-THIRD-PARTY).
