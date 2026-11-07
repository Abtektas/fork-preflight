# SPDX-License-Identifier: BSD-3-Clause
"""Generate data/releases/<release>.json for every release in releases.json.

For each release the PX4 source tree is extracted with `git archive` from a
local clone, and the facts come from the tree's own tools:

- Kconfig symbols: kconfiglib loads the tree's root Kconfig with the
  environment PX4's cmake/kconfig.cmake sets, including the Zenoh topic
  catalog that cmake generates at configure time. Board Kconfig files that
  the root Kconfig sources per board add board symbols.
- Commands: the MAIN argument of every px4_add_module() call, and the
  POSIX shell builtins platforms/common/apps.cpp.in registers (sleep ...).
- Parameters: the param-lineage snapshot of the release (names) and its
  combined history (removals with their successors), at a pinned commit.
- Removed Kconfig symbols: every symbol the previous data release defines and
  this one does not, with the commits that removed it and the successors
  PX4's git history shows (kconfig_removed.py). Needs the full history.

Usage:
  python generator/generate.py --px4 PATH --param-lineage PATH [--release R ...]
  python generator/generate.py --px4 PATH --param-lineage PATH --check
"""

from __future__ import annotations

import argparse
import glob
import io
import json
import os
import re
import subprocess
import sys
import tarfile
import tempfile
from importlib import metadata

# -I leaves the script's directory off sys.path.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from kconfig_removed import History, HistoryError, derive  # noqa: E402

FORMAT = 2
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GENERATOR_DIR = os.path.join(ROOT, "generator")
DATA_DIR = os.path.join(ROOT, "data", "releases")
SOURCE_REPOSITORY = "https://github.com/PX4/PX4-Autopilot"
TOOLS = ("kconfiglib", "empy", "pyros-genmsg")

# Variables the Kconfig files may use, and how they are set when the global
# symbol set is loaded. cmake/kconfig.cmake sets them per board.
KNOWN_MACROS = {"VENDOR", "MODEL", "LABEL", "ZENOH_KCONFIG_TOPICS"}
BOARD_SOURCE = 'osource "boards/$(VENDOR)/$(MODEL)/Kconfig"'
ZENOH_CATALOG = os.path.join("src", "modules", "zenoh", "Kconfig.topics")
MACRO = re.compile(r"\$\(([A-Za-z_][A-Za-z0-9_]*)\)")
COMMAND = re.compile(r"^[A-Za-z0-9_]+$")
# The POSIX shell's own commands, registered next to the module commands.
BUILTINS = "platforms/common/apps.cpp.in"
BUILTIN = re.compile(r'^\s*apps\["([^"]*)"\]\s*=', re.MULTILINE)


class GenerateError(Exception):
    pass


def version_key(release: str) -> tuple:
    """Release order: tags by number, alpha < beta < rc < final, main last."""
    if release == "main":
        return (1,)
    m = re.match(r"^v(\d+)\.(\d+)\.(\d+)(?:-(alpha|beta|rc)(\d+))?$", release)
    if not m:
        raise GenerateError(f"not a release name: {release}")
    pre = {"alpha": 0, "beta": 1, "rc": 2, None: 3}[m.group(4)]
    return (0, int(m.group(1)), int(m.group(2)), int(m.group(3)), pre, int(m.group(5) or 0))


def git_version() -> str:
    out = subprocess.run(["git", "--version"], capture_output=True, text=True).stdout.strip()
    m = re.match(r"^git version (\S+)", out)
    if not m:
        raise GenerateError(f"cannot read the git version from {out!r}")
    return m.group(1)


def git(px4: str, *args: str) -> bytes:
    result = subprocess.run(["git", "-C", px4, *args], capture_output=True)
    if result.returncode != 0:
        raise GenerateError(f"git {' '.join(args)}: {result.stderr.decode().strip()}")
    return result.stdout


def resolve(px4: str, release: str, commit: str) -> None:
    """The pinned commit must exist, and a tag must still point at it."""
    git(px4, "cat-file", "-e", f"{commit}^{{commit}}")
    if release != "main":
        tagged = git(px4, "rev-parse", f"refs/tags/{release}^{{commit}}").decode().strip()
        if tagged != commit:
            raise GenerateError(f"tag {release} is {tagged}, releases.json pins {commit}")


def extract(px4: str, commit: str, dest: str) -> None:
    archive = git(px4, "archive", "--format=tar", commit)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        tar.extractall(dest, filter="data")


def kconfig_files(tree: str) -> list[str]:
    found = []
    for base, dirs, files in os.walk(tree):
        dirs.sort()
        for name in sorted(files):
            if name.startswith("Kconfig"):
                found.append(os.path.join(base, name))
    return found


def macros(tree: str) -> set[str]:
    used = set()
    for path in kconfig_files(tree):
        with open(path, encoding="utf-8", errors="replace") as f:
            used.update(MACRO.findall(f.read()))
    return used


def generate_zenoh_catalog(tree: str, out: str) -> None:
    """Run the tree's own generator the way cmake/kconfig.cmake does."""
    msgs = sorted(glob.glob(os.path.join(tree, "msg", "*.msg")) + glob.glob(os.path.join(tree, "msg", "versioned", "*.msg")))
    os.makedirs(os.path.join(out, os.path.dirname(ZENOH_CATALOG)), exist_ok=True)
    result = subprocess.run(
        [
            sys.executable, "-I", os.path.join(tree, "Tools", "zenoh", "px_generate_zenoh_topic_files.py"),
            "--zenoh-config", "-f", *msgs,
            "-o", os.path.join(out, os.path.dirname(ZENOH_CATALOG)) + os.sep,
            "-e", os.path.join(tree, "Tools", "zenoh", "templates", "zenoh"),
        ],
        cwd=tree, capture_output=True, env=clean_env({}),
    )
    if result.returncode != 0 or not os.path.isfile(os.path.join(out, ZENOH_CATALOG)):
        raise GenerateError(f"Zenoh catalog generation failed: {result.stderr.decode()[-2000:]}")


def clean_env(extra: dict[str, str]) -> dict[str, str]:
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LC_ALL": "C"}
    env.update(extra)
    return env


def load_kconfig(tree: str, variables: dict[str, str]) -> dict:
    result = subprocess.run(
        [sys.executable, "-I", os.path.join(GENERATOR_DIR, "kconfig_dump.py")],
        cwd=tree, capture_output=True, env=clean_env({"srctree": tree, **variables}),
    )
    if result.returncode != 0:
        raise GenerateError(f"kconfiglib failed: {result.stderr.decode()[-2000:]}")
    loaded = json.loads(result.stdout)
    # Warnings name files by absolute path; keep them relative to the tree.
    loaded["warnings"] = sorted(w.replace(tree + os.sep, "") for w in loaded["warnings"])
    return loaded


def kconfig_data(tree: str, generated: str) -> dict:
    used = macros(tree)
    unknown = sorted(used - KNOWN_MACROS)
    if unknown:
        raise GenerateError(f"Kconfig uses variables the generator does not know: {', '.join(unknown)}")
    variables = {"VENDOR": "", "MODEL": "", "LABEL": ""}
    generated_files = []
    if "ZENOH_KCONFIG_TOPICS" in used:
        generate_zenoh_catalog(tree, generated)
        variables["ZENOH_KCONFIG_TOPICS"] = os.path.join(generated, ZENOH_CATALOG)
        generated_files.append(ZENOH_CATALOG.replace(os.sep, "/"))
    common = load_kconfig(tree, variables)
    symbols = set(common["symbols"])
    warnings = list(common["warnings"])

    board_symbols = {}
    with open(os.path.join(tree, "Kconfig"), encoding="utf-8") as f:
        per_board = BOARD_SOURCE in f.read()
    if per_board:
        for path in sorted(glob.glob(os.path.join(tree, "boards", "*", "*", "Kconfig"))):
            vendor, model = os.path.relpath(path, tree).split(os.sep)[1:3]
            board = load_kconfig(tree, {**variables, "VENDOR": vendor, "MODEL": model})
            missing = symbols - set(board["symbols"])
            if missing:
                raise GenerateError(f"board {vendor}/{model} loses symbols: {sorted(missing)[:5]}")
            extra = sorted(set(board["symbols"]) - symbols)
            if extra:
                board_symbols[f"{vendor}/{model}"] = extra
            warnings.extend(w for w in board["warnings"] if w not in warnings)
    return {
        "symbols": sorted(symbols),
        "board_symbols": board_symbols,
        "generated": generated_files,
        "warnings": sorted(warnings),
    }


def strip_cmake_comments(text: str) -> str:
    lines = []
    for line in text.splitlines():
        quoted = False
        for i, ch in enumerate(line):
            if ch == '"':
                quoted = not quoted
            elif ch == "#" and not quoted:
                line = line[:i]
                break
        lines.append(line)
    return "\n".join(lines)


def module_calls(text: str) -> list[str]:
    """The argument text of every px4_add_module( ... ) call, parentheses balanced."""
    calls = []
    for match in re.finditer(r"\bpx4_add_module\s*\(", text):
        depth, start = 1, match.end()
        for i in range(start, len(text)):
            if text[i] == "(":
                depth += 1
            elif text[i] == ")":
                depth -= 1
                if depth == 0:
                    calls.append(text[start:i])
                    break
        else:
            raise GenerateError("unbalanced px4_add_module(")
    return calls


def commands_data(tree: str) -> dict[str, list[str]]:
    commands: dict[str, set[str]] = {}
    for base, dirs, files in os.walk(tree):
        dirs.sort()
        if "CMakeLists.txt" not in files:
            continue
        path = os.path.join(base, "CMakeLists.txt")
        rel = os.path.relpath(path, tree).replace(os.sep, "/")
        with open(path, encoding="utf-8", errors="replace") as f:
            text = strip_cmake_comments(f.read())
        try:
            calls = module_calls(text)
        except GenerateError as e:
            raise GenerateError(f"{rel}: {e}") from None
        for call in calls:
            tokens = call.split()
            for i, token in enumerate(tokens[:-1]):
                if token == "MAIN":
                    name = tokens[i + 1]
                    if not COMMAND.match(name):
                        raise GenerateError(f"{rel}: MAIN {name} is not a literal command name")
                    commands.setdefault(name, set()).add(rel)
    for name in builtin_commands(tree):
        commands.setdefault(name, set()).add(BUILTINS)
    return {name: sorted(paths) for name, paths in sorted(commands.items())}


def builtin_commands(tree: str) -> list[str]:
    """Names apps.cpp.in registers as builtins (`apps["sleep"] = sleep_main;`). Scripts can
    run them although no px4_add_module() declares them; a module that becomes a builtin
    is not a removed command."""
    path = os.path.join(tree, *BUILTINS.split("/"))
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8", errors="replace") as f:
        names = BUILTIN.findall(f.read())
    for name in names:
        if not COMMAND.match(name):
            raise GenerateError(f"{BUILTINS}: builtin {name} is not a literal command name")
    return names


def parameters_data(lineage: str, pin: dict, release: str) -> tuple[dict | None, list[str]]:
    snapshot = os.path.join(lineage, "data", "snapshots", f"{release}.json")
    if not tracked(lineage, snapshot):
        return None, [f"Parameters: param-lineage at {pin['commit'][:10]} has no snapshot of {release}."]
    with open(snapshot, encoding="utf-8") as f:
        snap = json.load(f)
    histories_path = os.path.join(lineage, "data", "params.json")
    if not tracked(lineage, histories_path):
        raise GenerateError("param-lineage data/params.json is not tracked")
    with open(histories_path, encoding="utf-8") as f:
        histories = json.load(f)["parameters"]
    removed = {}
    for name, history in sorted(histories.items()):
        for entry in history["removed"]:
            if entry["in"] == release:
                # kind and successors exist only where param-lineage has a curated entry.
                removed[name] = {key: value for key, value in entry.items() if key != "in"}
    return {
        "source": {"repository": pin["repository"], "commit": pin["commit"]},
        "names": sorted(snap["parameters"]),
        "removed": removed,
    }, []


def copyright_line(tree: str) -> str:
    with open(os.path.join(tree, "LICENSE"), encoding="utf-8") as f:
        for line in f:
            if line.startswith("Copyright"):
                return line.strip()
    raise GenerateError("no Copyright line in LICENSE")


def check_lineage(lineage: str, pin: dict) -> None:
    head = subprocess.run(["git", "-C", lineage, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    if head != pin["commit"]:
        raise GenerateError(f"param-lineage is at {head or '?'}, releases.json pins {pin['commit']}")
    dirty = subprocess.run(["git", "-C", lineage, "status", "--porcelain", "--untracked-files=no", "--", "data"],
                           capture_output=True, text=True).stdout
    if dirty:
        raise GenerateError("param-lineage has local changes under data/")


def tracked(lineage: str, path: str) -> bool:
    """Untracked files are not part of the pinned commit, so they must not be read."""
    rel = os.path.relpath(path, lineage)
    return subprocess.run(["git", "-C", lineage, "ls-files", "--error-unmatch", "--", rel], capture_output=True).returncode == 0


def release_kconfig(px4: str, commit: str) -> dict:
    with tempfile.TemporaryDirectory(prefix="fork-preflight-") as tmp:
        tree = os.path.join(tmp, "tree")
        os.makedirs(tree)
        extract(px4, commit, tree)
        return kconfig_data(tree, os.path.join(tmp, "generated"))


def removed_data(px4: str, kconfig: dict, commit: str, previous: dict | None, symbols: dict[str, set[str]]) -> dict | None:
    """Symbols the previous data release defines and this one does not; null for the first release."""
    if previous is None:
        return None
    name = previous["release"]
    if name not in symbols:
        resolve(px4, name, previous["commit"])
        symbols[name] = set(release_kconfig(px4, previous["commit"])["symbols"])
    try:
        removed = derive(History(px4), symbols[name], previous["commit"], set(kconfig["symbols"]), commit)
    except HistoryError as e:
        raise GenerateError(str(e)) from None
    return {"previous": name, "symbols": removed}


def build_release(px4: str, lineage: str, pin: dict, release: str, commit: str,
                  previous: dict | None = None, symbols: dict[str, set[str]] | None = None) -> dict:
    """One release's data. `symbols` caches global Kconfig symbols per release across calls."""
    symbols = {} if symbols is None else symbols
    resolve(px4, release, commit)
    with tempfile.TemporaryDirectory(prefix="fork-preflight-") as tmp:
        tree = os.path.join(tmp, "tree")
        generated = os.path.join(tmp, "generated")
        os.makedirs(tree)
        extract(px4, commit, tree)
        parameters, notes = parameters_data(lineage, pin, release)
        kconfig = kconfig_data(tree, generated)
        symbols[release] = set(kconfig["symbols"])
        kconfig["removed"] = removed_data(px4, kconfig, commit, previous, symbols)
        return {
            "format": FORMAT,
            "release": release,
            "source": {"repository": SOURCE_REPOSITORY, "commit": commit},
            "license": {
                "license": "BSD-3-Clause",
                "copyright": copyright_line(tree),
                "text": "LICENSE-THIRD-PARTY",
            },
            "tools": {**{name: metadata.version(name) for name in TOOLS}, "git": git_version()},
            "kconfig": kconfig,
            "commands": commands_data(tree),
            "parameters": parameters,
            "notes": notes,
        }


def compare(path: str, data: dict, text: str) -> tuple[bool, str]:
    """Byte comparison with the file. Only the recorded git version may differ: it is noted,
    since git is not pinned and every value it produces is still compared."""
    try:
        with open(path, encoding="utf-8") as f:
            stored = f.read()
    except FileNotFoundError:
        return False, ""
    if stored == text:
        return True, ""
    try:
        made_with = json.loads(stored)["tools"]["git"]
    except (ValueError, KeyError, TypeError):
        return False, ""
    here = data["tools"]["git"]
    if made_with != here and stored == dumps({**data, "tools": {**data["tools"], "git": made_with}}):
        return True, f" (data made with git {made_with}, this is git {here})"
    return False, ""


def dumps(data: dict) -> str:
    return json.dumps(data, indent=1, sort_keys=True, ensure_ascii=False) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--px4", required=True, help="local clone of PX4-Autopilot with the release tags")
    parser.add_argument("--param-lineage", required=True, help="local clone of param-lineage at the pinned commit")
    parser.add_argument("--release", action="append", help="only this release (repeatable)")
    parser.add_argument("--check", action="store_true", help="regenerate and compare with data/releases, write nothing")
    parser.add_argument("--out", default=DATA_DIR, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)

    with open(os.path.join(GENERATOR_DIR, "releases.json"), encoding="utf-8") as f:
        manifest = json.load(f)
    pin = manifest["parameter_data"]
    entries = manifest["releases"]
    try:
        ordered = sorted(entries, key=lambda e: version_key(e["release"]))
    except GenerateError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    if ordered != entries:
        print("error: releases.json must list the releases in version order", file=sys.stderr)
        return 2
    previous = {e["release"]: p for p, e in zip([None, *entries], entries)}
    if args.release:
        known = {e["release"] for e in entries}
        unknown = [r for r in args.release if r not in known]
        if unknown:
            print(f"error: not in releases.json: {', '.join(unknown)}", file=sys.stderr)
            return 2
        entries = [e for e in entries if e["release"] in args.release]

    try:
        check_lineage(args.param_lineage, pin)
        try:
            History(args.px4).check_full()
        except HistoryError as e:
            raise GenerateError(str(e)) from None
        differ = []
        symbols: dict[str, set[str]] = {}
        for entry in entries:
            data = build_release(args.px4, args.param_lineage, pin, entry["release"], entry["commit"],
                                 previous[entry["release"]], symbols)
            text = dumps(data)
            path = os.path.join(args.out, f"{entry['release']}.json")
            if args.check:
                same, note = compare(path, data, text)
                print(f"{entry['release']}: {'identical' if same else 'DIFFERS'}{note}")
                if not same:
                    differ.append(entry["release"])
            else:
                os.makedirs(args.out, exist_ok=True)
                with open(path, "w", encoding="utf-8", newline="\n") as f:
                    f.write(text)
                print(f"{entry['release']}: wrote {os.path.relpath(path, ROOT)}")
    except GenerateError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    return 1 if differ else 0


if __name__ == "__main__":
    sys.exit(main())
