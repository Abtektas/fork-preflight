# SPDX-License-Identifier: BSD-3-Clause
"""Kconfig symbols a release removed, with what PX4's own git history says replaced them.

For a release R and the data release P before it, every symbol defined at P and not
at R gets:

  removed_by  up to 3 commits in P..R that delete its `config` line (or, for a Zenoh
              topic, its msg file), with their subjects
  successors  from the first rule that matches:
    renamed   K1 a `config Y` the same commit adds, Y new at R, and a batch prefix
                 rename (>= 3 pairs P1_S -> P2_S in that commit) or a unique name
                 match; up to 3 hops through symbols added and dropped inside P..R
                 (`via`). Veto: Y (or a hop) must replace X in >= 50% of the in-tree
                 board commits that dropped X.
              K4 USER_<S> follows the successor of MODULES_<S>
              Z1 git rename (-M50%) of the topic's msg file in its deleting commit
    boards    K2 in-tree boards: (a) same commit, Y added where X was dropped in
                 >= 80% of X's board edits (>= 2) and >= 50% of Y's adds; (b) net
                 P -> R, >= 80% of boards that set X set Y, >= 80% of boards gaining
                 Y had X, and Y's `config` came from a commit touching X's directory
    selected  K3 X selected Y at P and R defines Y
  and, only for a symbol none of these gives a successor:
    renamed   K5 the commit that drops X renames siblings (a git msg rename, or a batch
                 prefix rename) with the token change A -> B, and adds X with A -> B
              K7 X's Kconfig file was orphaned before P (no other file left in its
                 directory): a unique name match among the `config` lines added by
                 the commit that deleted the directory's other files
  K1 and K7 take, when difflib cannot decide, the one candidate that inserts tokens, lengthens
  the last token or names X's part family (ICP10100 -> ICP101XX).
    named     K6 the message of the commit that drops X says "rename A to B" or
                 "from A to B" (X with A -> B, or B itself, defined at R), or "equivalent
                 to", "replaced by/with", "in favour of", "superseded by" B, where B is
                 the end of exactly one symbol at R that shares X's first two tokens

The rules and thresholds were measured on the data releases and on v1.13.3 -> v1.14.0
(contrib notes); they read only git objects of the pinned commits, so the output is
reproducible from a full (not shallow) clone. Rename scoring is git's, so the data
records the git version.
"""

from __future__ import annotations

import difflib
import os
import re
import subprocess
from collections import Counter

CONFIG_RE = re.compile(r"^\s*(?:menu)?config\s+([A-Za-z0-9_]+)\s*$")
SET_RE = re.compile(r"^CONFIG_([A-Za-z0-9_]+)=(.*)$")
NOT_SET_RE = re.compile(r"^# CONFIG_([A-Za-z0-9_]+) is not set")
BLOCK_RE = re.compile(r"^\s*(choice|endchoice|menu|endmenu|if|endif|comment|source|rsource)\b")
SELECT_RE = re.compile(r"^\s*select\s+([A-Za-z0-9_]+)")
GENERATED = {"src/modules/zenoh/Kconfig.topics"}
ZENOH = "ZENOH_PUBSUB_"
SUBJECT_LENGTH = 100
MAX_COMMITS = 3
MAX_HOPS = 3
WORD = r"([A-Za-z0-9_]+)"
RENAME_RES = (re.compile(rf"\brenam(?:e|es|ed|ing)\s+{WORD}\s+to\s+{WORD}", re.I),
              re.compile(rf"\bfrom\s+(?:old\s+)?{WORD}\s+to\s+(?:new\s+)?{WORD}", re.I))
REPLACED_RE = re.compile(rf"\b(?:equivalent\s+to|replaced\s+(?:by|with)|in\s+favou?r\s+of|superseded\s+by)"
                         rf"\s+(?:the\s+)?{WORD}", re.I)


GIT_OPTIONS = ("-c", "color.ui=never", "-c", "diff.noprefix=false", "-c", "diff.mnemonicPrefix=false",
               "-c", "diff.relative=false", "-c", "log.showSignature=false", "-c", "core.quotePath=true")
GIT_ENV = {**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull, "LC_ALL": "C"}


class HistoryError(Exception):
    pass


class History:
    """Read-only queries on a PX4 clone."""

    def __init__(self, px4: str):
        self.px4 = px4

    def git(self, *args: str, ok: tuple[int, ...] = (0,)) -> str:
        # Diff and log text is parsed, so user and system git settings must not change it.
        result = subprocess.run(["git", "-C", self.px4, *GIT_OPTIONS, *args], capture_output=True, env=GIT_ENV)
        if result.returncode not in ok:
            raise HistoryError(f"git {' '.join(args)}: {result.stderr.decode().strip()}")
        return result.stdout.decode("utf-8", errors="replace")

    def check_full(self) -> None:
        if self.git("rev-parse", "--is-shallow-repository").strip() != "false":
            raise HistoryError("the PX4 clone is shallow; the removal history needs the full history")

    def message(self, commit: str) -> str:
        return self.git("log", "-1", "--format=%B", commit)

    def commit_renames(self, commit: str) -> tuple[list[tuple[str, str]], set[str]]:
        """Renames one commit makes: git msg renames (-M50%) and batch prefix `config` renames,
        as (old, new) symbol names; and the Zenoh topics of the msg files it adds."""
        pairs, added = [], set()
        for line in self.git("show", "-M50%", "--format=", "--name-status", commit, "--", "msg").splitlines():
            parts = line.split("\t")
            if parts[0].startswith("R") and parts[1].endswith(".msg") and parts[2].endswith(".msg"):
                pairs.append((topic_of(parts[1]), topic_of(parts[2])))
            elif parts[0] == "A" and parts[1].endswith(".msg"):
                added.add(topic_of(parts[1]))
        rem, add = self.commit_configs(commit)
        pairs += sorted(prefix_groups(rem - add, add - rem).items())
        return pairs, added | (add - rem)

    def commit_configs(self, commit: str) -> tuple[set[str], set[str]]:
        """`config` names one commit drops and adds in non-generated Kconfig files."""
        text = self.git("show", "--format=", "-U0", "--no-renames", commit, "--", "*Kconfig*")
        cur, rem, add = None, set(), set()
        for line in text.splitlines():
            if line.startswith("diff --git"):
                cur = line.split(" b/", 1)[1]
            elif line.startswith(("---", "+++")) or cur in GENERATED:
                continue
            elif line[:1] in "+-":
                m = CONFIG_RE.match(line[1:])
                if m:
                    (rem if line[0] == "-" else add).add(m.group(1))
        return rem, add

    def orphan_origin(self, p: str, x: str) -> str | None:
        """When the directory of x's Kconfig file holds nothing else at p, the commit before p
        that deleted the directory's other files."""
        for f in self.definition_files(p, x):
            d = os.path.dirname(f)
            if d.count("/") < 2:
                continue
            left = self.git("ls-tree", "-r", "--name-only", p, "--", d + "/").split()
            if left and all(os.path.basename(t).startswith("Kconfig") for t in left):
                c = self.git("log", "-1", "--format=%H", "--diff-filter=D", p, "--", d + "/").strip()
                if c:
                    return c
        return None

    def subject(self, commit: str) -> str:
        return self.git("log", "-1", "--format=%s", commit).strip()[:SUBJECT_LENGTH]

    def boards(self, rev: str) -> dict[str, set[str]]:
        out = {}
        for path in self.git("ls-tree", "-r", "--name-only", rev, "boards").split():
            if path.endswith(".px4board"):
                out[path] = {m.group(1) for m in map(SET_RE.match, self.git("show", f"{rev}:{path}").splitlines())
                             if m and m.group(2) not in ("n", '""')}
        return out

    def definition_files(self, rev: str, symbol: str) -> list[str]:
        pattern = rf"^[[:space:]]*(menu)?config[[:space:]]+{symbol}[[:space:]]*$"
        hits = self.git("grep", "-l", "-E", pattern, rev, "--", "*Kconfig*", ok=(0, 1)).split()
        return [hit.split(":", 1)[1] for hit in hits]

    def selects(self, rev: str, symbol: str) -> list[str]:
        out = []
        for path in self.definition_files(rev, symbol):
            inside = False
            for line in self.git("show", f"{rev}:{path}").splitlines():
                m = CONFIG_RE.match(line)
                if m or BLOCK_RE.match(line):
                    inside = bool(m) and m.group(1) == symbol
                    continue
                m = SELECT_RE.match(line)
                if inside and m:
                    out.append(m.group(1))
        return out

    def config_removals(self, p: str, r: str) -> dict[str, list[tuple[str, set[str], str | None]]]:
        """Per X: commits in p..r (no merges) dropping `config X` from a non-generated Kconfig
        file, with the `config` names each adds and X's batch-rename partner."""
        found: dict[str, list[tuple[str, set[str], str | None]]] = {}
        for c in self.git("rev-list", "--no-merges", f"{p}..{r}", "--", "*Kconfig*").split():
            rem, add = self.commit_configs(c)
            batch = prefix_groups(rem - add, add - rem)
            for x in rem - add:
                found.setdefault(x, []).append((c, add, batch.get(x)))
        return found

    def board_cochange(self, p: str, r: str) -> tuple[Counter, dict[str, Counter], Counter]:
        """Per (commit, board file) in p..r: symbols dropped, and those added in the same change."""
        return self.board_changes(self.git("rev-list", "--no-merges", f"{p}..{r}", "--", "*.px4board").split())

    def board_changes(self, commits: list[str]) -> tuple[Counter, dict[str, Counter], Counter]:
        dropped, together, adds = Counter(), {}, Counter()
        for c in commits:
            text = self.git("show", "--format=", "-U0", "--no-renames", c, "--", "*.px4board")
            files: dict[str, tuple[set[str], set[str]]] = {}
            cur = None
            for line in text.splitlines():
                if line.startswith("diff --git"):
                    cur = line.split(" b/", 1)[1]
                    files[cur] = (set(), set())
                elif line.startswith(("---", "+++")):
                    continue
                elif line[:1] in "+-":
                    m = SET_RE.match(line[1:]) or NOT_SET_RE.match(line[1:])
                    if m:
                        files[cur][0 if line[0] == "-" else 1].add(m.group(1))
            for rs, ad in files.values():
                for y in ad - rs:
                    adds[y] += 1
                for x in rs - ad:
                    dropped[x] += 1
                    for y in ad - rs:
                        together.setdefault(x, Counter())[y] += 1
        return dropped, together, adds

    def msg_history(self, r: str, x: str) -> tuple[str | None, str | None]:
        """Deleting commit (reachable from r) of the msg file whose topic is x, and its git rename."""
        camel = "".join(w.capitalize() for w in x[len(ZENOH):].lower().split("_"))
        paths = [f"msg/{camel}.msg", f"msg/versioned/{camel}.msg"]
        c = self.git("log", "-1", "--format=%H", "--diff-filter=D", r, "--", *paths).strip()
        if not c:
            return None, None
        for line in self.git("show", "-M50%", "--format=", "--name-status", c, "--", "msg").splitlines():
            parts = line.split("\t")
            if parts[0].startswith("R") and parts[1] in paths:
                return c, parts[2]
        return c, None

    def same_origin(self, p: str, r: str, x: str, y: str) -> bool:
        """Whether a commit in p..r that adds `config y` touches the directory defining x at p."""
        dirs = [os.path.dirname(f) + "/" for f in self.definition_files(p, x) if os.path.dirname(f).count("/") >= 2]
        if not dirs:
            return False
        pattern = rf"^[[:space:]]*(menu)?config[[:space:]]+{y}[[:space:]]*$"
        for c in self.git("log", "--format=%H", "-G", pattern, f"{p}..{r}", "--", "*Kconfig*").split():
            touched = self.git("show", "--format=", "--name-only", "--no-renames", c).split()
            if any(t.startswith(d) for t in touched for d in dirs):
                return True
        return False


def snake(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def topic_of(path: str) -> str:
    return ZENOH + snake(os.path.basename(path)[:-4]).upper()


def insert_rename(x: str, y: str) -> bool:
    """Y is X with tokens inserted, same last token (DRIVERS_IMU_L3GD20 -> DRIVERS_IMU_ST_L3GD20)."""
    a, b = x.split("_"), y.split("_")
    if len(b) <= len(a) or a[-1] != b[-1]:
        return False
    i = 0
    for t in b:
        if i < len(a) and t == a[i]:
            i += 1
    return i == len(a)


def extends_last(x: str, y: str) -> bool:
    """Y is X with letters appended to its last token (..._MS4525 -> ..._MS4525DO)."""
    a, b = x.split("_"), y.split("_")
    return len(a) == len(b) and a[:-1] == b[:-1] and b[-1].startswith(a[-1]) and b[-1] != a[-1]


def family_match(x: str, y: str) -> bool:
    """Y names a part family that covers X: same tokens, last token of equal length where Y has
    X in some places (..._ICP10100 -> ..._ICP101XX)."""
    a, b = x.split("_"), y.split("_")
    return (len(a) == len(b) and a[:-1] == b[:-1] and len(a[-1]) == len(b[-1]) and "X" in b[-1]
            and a[-1] != b[-1] and all(i == j or j == "X" for i, j in zip(a[-1], b[-1])))


def substitute(x: str, old: str, new: str) -> str | None:
    """X with the token run `old` replaced by `new` (whole tokens, first occurrence)."""
    xt, ot = x.split("_"), old.split("_")
    for i in range(len(xt) - len(ot) + 1):
        if xt[i:i + len(ot)] == ot:
            return "_".join(xt[:i] + new.split("_") + xt[i + len(ot):])
    return None


def token_change(old: str, new: str) -> tuple[str, str] | None:
    """The token run that differs between two names, after their common prefix and suffix."""
    a, b = old.split("_"), new.split("_")
    i = 0
    while i < min(len(a), len(b)) and a[i] == b[i]:
        i += 1
    j = 0
    while j < min(len(a), len(b)) - i and a[-1 - j] == b[-1 - j]:
        j += 1
    if i + j >= len(a) or i + j >= len(b):
        return None
    return "_".join(a[i:len(a) - j]), "_".join(b[i:len(b) - j])


def norm(word: str) -> str:
    word = word.upper().strip("_")
    return word[len("CONFIG_"):] if word.startswith("CONFIG_") else word


def name_match(x: str, y: str) -> bool:
    return insert_rename(x, y) or difflib.SequenceMatcher(None, x, y).ratio() >= 0.8


def prefix_groups(rem: set[str], add: set[str]) -> dict[str, str]:
    """X = P1_S, Y = P2_S in one commit, with >= 3 pairs sharing (P1, P2): a batch prefix rename."""
    groups: dict[tuple[str, str], list[tuple[str, str]]] = {}
    for x in sorted(rem):
        xt = x.split("_")
        for y in sorted(add):
            yt = y.split("_")
            n = 0
            while n < min(len(xt), len(yt)) and xt[-1 - n] == yt[-1 - n]:
                n += 1
            if n == 0 or n == len(xt) or n == len(yt):
                continue
            groups.setdefault(("_".join(xt[:-n]), "_".join(yt[:-n])), []).append((x, y))
    out: dict[str, set[str]] = {}
    for pairs in groups.values():
        xs = [x for x, _ in pairs]
        if len(pairs) >= 3 and len(set(xs)) == len(xs):
            for x, y in pairs:
                out.setdefault(x, set()).add(y)
    return {x: ys.pop() for x, ys in out.items() if len(ys) == 1}


def k1_step(x: str, removals: dict, pool: set[str]) -> str | None:
    """One K1 hop: a unique name-matched `config` added by a commit that drops x."""
    batch = {b for _, _, b in removals.get(x, []) if b and b in pool}
    if len(batch) == 1:
        return batch.pop()
    added = {y for _, add, _ in removals.get(x, []) for y in add if y in pool and name_match(x, y)}
    cands = sorted(((difflib.SequenceMatcher(None, x, y).ratio(), y) for y in added), reverse=True)
    if cands and (len(cands) == 1 or cands[0][0] - cands[1][0] >= 0.03):
        return cands[0][1]
    structural = [y for _, y in cands if insert_rename(x, y) or extends_last(x, y) or family_match(x, y)]
    return structural[0] if len(structural) == 1 else None


def board_step(x: str, dropped: Counter, together: dict, adds: Counter, pool: set[str]) -> str | None:
    """One K2a hop: the single Y that replaced x in the same board commits."""
    co = together.get(x, Counter())
    ys = [y for y, k in co.items() if y in pool and dropped[x] >= 2 and k / dropped[x] >= 0.8 and k / adds[y] >= 0.5
          and all(o == x or together[o][y] <= k for o in together)]
    return ys[0] if len(ys) == 1 else None


def k1_chain(x: str, removals: dict, new: set[str], at_r: set[str], boards: tuple) -> list[str] | None:
    """Follow hops through symbols added and dropped again inside the interval. The first hop
    is K1; a later hop, or one to a transient symbol, may also come from board commits."""
    chain, cur = [], x
    transient = {y for y in removals if y not in at_r}
    for _ in range(MAX_HOPS):
        y = k1_step(cur, removals, new | transient)
        if not y and cur != x:
            y = board_step(cur, *boards, new)
        if not y:
            t = board_step(cur, *boards, transient)
            y = t if t and t not in new else None
        if not y or y in chain or y == x:
            return None
        chain.append(y)
        if y in new:
            return chain
        cur = y
    return None


def own_boards_agree(history: History, x: str, chain: list[str], removals: dict) -> bool:
    """For a one-hop rename that only lengthens the last token (..._MS4525 -> ..._MS4525DO), the
    board veto counts only the board edits of the commit that renames it: other commits may drop
    x for an unrelated reason (a common menu instead of single drivers). Not for token inserts:
    DRIVERS_OSD -> DRIVERS_OSD_MSP_OSD is a new driver, not a rename."""
    if len(chain) != 1 or not extends_last(x, chain[0]):
        return False
    commits = [c for c, add, _ in removals.get(x, []) if chain[0] in add]
    if not commits:
        return False
    dropped, together, _ = history.board_changes(commits)
    return dropped[x] == 0 or together.get(x, Counter())[chain[0]] / dropped[x] >= 0.5


def late_successor(history: History, p: str, x: str, commits: list[str],
                   r_symbols: set[str]) -> tuple[str, list[str]] | None:
    """K5, K6, K7 for a symbol the main rules left without a successor."""
    zenoh = x.startswith(ZENOH)
    # K5: a sibling rename's token change, applied to x, gives a name the same commit adds.
    for c in commits[:MAX_COMMITS]:
        pairs, added = history.commit_renames(c)
        ys = set()
        for old, new in pairs:
            change = token_change(old, new)
            y = change and substitute(x, *change)
            if y and y != x and y in added and y in r_symbols:
                ys.add(y)
        if len(ys) == 1:
            return "renamed", sorted(ys)
    # K6: the commit message names the successor.
    for c in commits[:MAX_COMMITS]:
        text = history.message(c)
        ys = set()
        for rx in RENAME_RES:
            for m in rx.finditer(text):
                old, new = norm(m.group(1)), norm(m.group(2))
                if old == x:
                    ys.add(new)
                elif zenoh:
                    t = substitute(x[len(ZENOH):], old, new)
                    ys.add(ZENOH + t if t else "")
                else:
                    ys.add(substitute(x, old, new) or "")
        ys = {y for y in ys if y and y != x and y in r_symbols}
        if not ys:
            head = x.split("_")[:2]
            for m in REPLACED_RE.finditer(text):
                end = norm(m.group(1)).split("_")
                ys |= {y for y in r_symbols if y != x and y.split("_")[:2] == head and len(y.split("_")) > len(end) + 1
                       and y.split("_")[-len(end):] == end}
        if len(ys) == 1:
            return "named", sorted(ys)
    # K7: x's Kconfig was orphaned; the commit that emptied its directory added the successor.
    if not zenoh:
        c = history.orphan_origin(p, x)
        if c:
            _, add = history.commit_configs(c)
            cands = {y for y in add if y in r_symbols and y != x and name_match(x, y)}
            if cands:
                y = k1_step(x, {x: [(c, cands, None)]}, cands)
                if y:
                    return "renamed", [y]
    return None


def derive(history: History, p_symbols: set[str], p: str, r_symbols: set[str], r: str) -> dict[str, dict]:
    """Every symbol defined at commit p and not at commit r, with its removal evidence."""
    removed, new = p_symbols - r_symbols, r_symbols - p_symbols
    if not removed:
        return {}
    removals = history.config_removals(p, r)
    bp, br = history.boards(p), history.boards(r)
    common = [b for b in bp if b in br]
    gained = {b: br[b] - bp[b] for b in common}
    base_rate = Counter(y for b in common for y in gained[b])
    dropped, together, adds = history.board_cochange(p, r)
    out: dict[str, dict] = {}
    for x in sorted(removed):
        entry: dict = {"removed_by": [], "evidence": None, "successors": []}
        commits = [c for c, _, _ in removals.get(x, [])]
        renamed_to = None
        if x.startswith(ZENOH):
            c, dest = history.msg_history(r, x)
            if c:
                commits = [c]
                if dest and topic_of(dest) in r_symbols:
                    renamed_to = [topic_of(dest)]
        else:
            chain = k1_chain(x, removals, new, r_symbols, (dropped, together, adds))
            if chain and not (dropped[x] == 0 or max(together.get(x, Counter())[h] for h in chain) / dropped[x] >= 0.5) \
                    and not own_boards_agree(history, x, chain, removals):
                chain = None
            if chain:
                renamed_to = [chain[-1]]
                if len(chain) > 1:
                    entry["via"] = chain[:-1]
        entry["removed_by"] = [{"commit": c, "subject": history.subject(c)} for c in commits[:MAX_COMMITS]]
        if renamed_to:
            entry["evidence"], entry["successors"] = "renamed", renamed_to
        else:
            co = together.get(x, Counter())
            # K2a: same-commit board change; Y goes to the removed symbol it co-changed with most.
            ys = [y for y, k in co.items() if y in new and dropped[x] >= 2 and k / dropped[x] >= 0.8 and k / adds[y] >= 0.5
                  and all(together.get(o, Counter())[y] <= k for o in removed if o != x)]
            # K2b: net board change, and Y's `config` came from a commit touching X's directory.
            had = [b for b in common if x in bp[b]]
            net = Counter(y for b in had for y in gained[b] if y in new)
            for y, k in sorted(net.items()):
                if y not in ys and len(had) >= 2 and k / len(had) >= 0.8 and k / base_rate[y] >= 0.8 \
                        and history.same_origin(p, r, x, y):
                    ys.append(y)
            if ys:
                entry["evidence"], entry["successors"] = "boards", sorted(ys)
                entry["boards"] = {
                    y: {"boards": co[y], "of": dropped[x], "how": "same-commit"}
                    if y in co and co[y] / dropped[x] >= 0.8 else
                    {"boards": net[y], "of": len(had), "how": "net"}
                    for y in sorted(ys)
                }
            else:
                selected = sorted({y for y in history.selects(p, x) if y in r_symbols})
                if selected:
                    entry["evidence"], entry["successors"] = "selected", selected
        if not entry["successors"]:
            late = late_successor(history, p, x, commits, r_symbols)
            if late:
                entry["evidence"], entry["successors"] = late
        out[x] = entry
    # K4: USER_<S> follows the successor of MODULES_<S>.
    for x, entry in out.items():
        if x.startswith("USER_") and not entry["successors"]:
            module = out.get("MODULES_" + x[len("USER_"):])
            if module:
                ys = ["USER_" + y[len("MODULES_"):] for y in module["successors"] if y.startswith("MODULES_")]
                ys = [y for y in ys if y in new]
                if ys:
                    entry["evidence"], entry["successors"] = "renamed", ys
    return out
