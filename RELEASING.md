# Releasing

How to add a PX4 release to the data and how to cut a fork-preflight
release. Commands run from the repository root, with the clones and tools
described under "Regenerate" and "Tests" in [README.md](README.md).

## Add a PX4 release to the data

1. Wait for the PX4 tag. Fetch it into a full (not shallow) PX4 clone:
   `git -C ../PX4-Autopilot fetch --tags origin`. Pin the commit the tag
   points to: `git -C ../PX4-Autopilot rev-parse 'refs/tags/<tag>^{commit}'`.
2. Parameters come from a param-lineage snapshot of the same release. When
   param-lineage has one, set `parameter_data.commit` in
   `generator/releases.json` to that param-lineage commit and check out the
   param-lineage clone at it.
3. Add `{"release": "<tag>", "commit": "<commit>"}` to `releases` in
   `generator/releases.json`, in version order (the generator refuses any
   other order). Keep the release candidates: forks may be based on them.
   To move `main`, change its commit in the same step.
4. Regenerate and check:

   ```sh
   .venv/bin/python -I generator/generate.py --px4 ../PX4-Autopilot --param-lineage ../param-lineage
   .venv/bin/python -I generator/generate.py --px4 ../PX4-Autopilot --param-lineage ../param-lineage --check
   ```

   `kconfig.removed` is relative to the previous data release, so adding a
   release also changes the file of the release after it (usually `main`).
   A new param-lineage commit can change older files too; read every changed
   file in `git diff --stat data/releases`.
5. Review `kconfig.removed` of the new release by hand: for each removed
   symbol, read the named commits and compare the successors with what they
   did. Write down right, wrong and missed. This history was not used to
   tune the rules, so it is the first clean measurement of them.
6. Update what names the newest release: the known-answer table in
   README.md, `target:` in the README's Action example, the tests in
   `crates/cli/tests/cli.rs`, `crates/core/src/data.rs` and
   `tests/test_generator.py` that should cover it, and the release list in
   README.md "Release data".

## Gates

Run all of them on the final tree, on macOS arm64 and on linux/amd64
(`rust:1.94.0` for the scanner, Python 3.12 for the generator). Print the
tool versions next to the results.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
FORK_PREFLIGHT_PX4="$PWD/../PX4-Autopilot" cargo test
.venv/bin/python -I -m unittest discover -s tests
.venv/bin/python -I generator/generate.py --px4 ../PX4-Autopilot --param-lineage ../param-lineage --check
shellcheck action/run.sh
```

`FORK_PREFLIGHT_PX4` must be an absolute path, or the known-answer tests on
PX4 are skipped. Also run the Action's steps once in a `rust:1.94.0`
linux/amd64 container and validate one SARIF report against the SARIF 2.1.0
schema.

## Cut a fork-preflight release

1. Set `version` in the workspace `Cargo.toml` and run
   `cargo update --workspace` so that `Cargo.lock` follows; check that
   `fork-preflight --version` prints it.
2. Replace the heading of the top section of `CHANGELOG.md` with the version
   and the date (`## 0.1.0 (YYYY-MM-DD)`), and make its first lines name the
   newest PX4 release in the data.
3. Update the "Status" paragraph of README.md.
4. Run the gates above on that commit.
5. Tag it: `git tag -a v<version> -m "fork-preflight <version>"`. Push the
   commit and the tag, then create the GitHub release from the tag with the
   CHANGELOG section as its text. Every release text keeps the notices of
   [DISCLAIMER.md](DISCLAIMER.md).
6. Action users pin a commit (`uses: Abtektas/fork-preflight@<commit>`);
   name the tagged commit in the release text.
