#!/usr/bin/env python3
"""Pre-triage of the fuzz runner's findings, against the known SQLite bugs of
known-bugs.json (python3, standard library only).

    pretriage.py [--all] [--force] [PATH ...]   # runs or findings dirs (default RUN_ROOT/current)
    pretriage.py --check-bugs                   # the nightly known-bug check
    pretriage.py --expected [BUG ...]           # a bug's check output on its patched CLI

Every finding example (`findings/<signature>/w<worker>-<n>/`, those with a
`message.txt`; crashes, hangs, timeouts and generator panics have none) that
has no `triage.json` is replayed with the CLIs build-clis.sh built
(RUN_ROOT/clis/current: `trunk/sqlite3` and one `<bug>/sqlite3` per open bug
with its fix applied), each on its own copy of `after.db`, one statement per
CLI process (`.mode quote`, a timeout per statement), and gets a verdict.

The replay is in the example's storage (`storage` in its example.json:
`memory` before there was any other): a memory worker's on a plain copy of
`after.db` (the in-memory database the runner serialized, which the CLI
opens as a file in rollback mode, as before); a WAL worker's (`+wal`
profiles, `storage: wal`) on a copy put in WAL mode first (`PRAGMA
journal_mode=WAL`, by a CLI process of its own; after.db is the in-memory
database for these too, so the WAL holds no frames of the case's
statements, and a failure that needs them is not reproduced); and a storage
oracle's comparison (`results differ (storage)` and its one-sided kinds,
original in memory, variant in a WAL file, the same SQL) with its original
on the CLI's `.open --deserialize` of a copy (in memory, as the runner's)
and its variant on a WAL-mode copy, compared by the same rules as any other
comparison (`infer_mode`: as the plans oracle compares). The case's
connection settings that change plans but are not in `after.db`
(`PRAGMA automatic_index` and `PRAGMA reverse_unordered_selects`, the last
of each among the statements in its `input.txt`, [`connection_pragmas`])
are run before every replayed statement, as the runner's queries ran after
them. after.db is the main database only, as committed, so a case that
leaves a TEMP or ATTACHed database or a transaction open (`case_needs`) is
replayed with its statements run before every statement (`case_replay`,
recorded as `case_replay` in triage.json): on the example's `before.db`
(the case's starting database) with its `case.sql`, as the runner opened
it (`faithful`); for an example from before the runner wrote them, on
after.db with input.txt's statements (`reconstructed`, approximate: for a
TEMP or ATTACHed database every statement with main read-only, else the
statements from the open transaction on). A replay that fails with "no such
table" or "no such column" where the run did not is on the wrong database:
undecided (`manual`), never `new`. The verdicts:

- `excluded`: the message's error (the original's, or one side's) is one the
  current oracles skip (`OVERFLOW`, `LIMITS`, `JSON_ERRORS`, the lists of
  ephyra-sqlite's sqlancer.rs; and `IO_ERRORS`, the host's storage failing,
  which the storage oracle skips and which is never SQLite's logic
  elsewhere), or a case's statement failed with "non-deterministic use of
  X() in Y" (`nondeterministic`: a date function reading `now` in an
  expression index, which the oracles skip since run 20261012-0317), or
  the query is one they skip as known
  SQLite behaviour (`known_issue`: invalid_text_pattern): a false positive
  of a stale binary.
- `jsonb-equivalent`: the message's own two results differ, but agree once
  a BLOB that is valid JSONB compares as `json(x)` (as the current oracles
  compare): an encoding difference, not a failure.
- `known` (`known:<bug>` in summaries; `bugs` lists every bug that explains
  it): trunk reproduces the failure and that bug's patched CLI makes it go
  away. Reproduces and goes away mean, by the message's kind:
  - a comparison (`results differ`, `the original fails, the variant does
    not`, `the variant fails, the original does not`): both sides run (a
    TLP variant's partitions one by one, rows concatenated; NoREC's NULL sum
    taken as 0; order-sorted's rows cut to the original's width;
    order-limit's original sliced) and compared as the oracle compares them
    (`infer_mode`, from the variant and the SQL: multiset, set, groups,
    equiv, ordered, scalar, sum), JSONB normalized, an error on one side
    that the oracles skip taken as agreement: they disagree on trunk, and
    agree on the patched CLI (not by both failing);
  - plans-noopt (`plans-noopt-<name>` when one optimization alone
    reproduces it; `noopt`), the query as written with optimizations off:
    the variant (the same SQL) replayed with the CLI's `.testctrl
    optimizations MASK` first, MASK from the message's last line
    (`  optimizations off: 0x00080000 (BloomFilter)`), on trunk and every
    patched CLI alike, so a known bug's fix is found as for any other
    comparison;
  - setops-def and window-def, whose reference side the runner computed
    in Rust from other queries' rows (a compound from its SELECTs run
    alone, `compound_parts`; the windows from their input query): that
    side is recomputed on each CLI from those queries' rows as the runner
    computes it (`setops_reference`, `windows_reference`), so both sides
    of the comparison are each CLI's, as for every other comparison:
    trunk's two sides disagree; the patched CLI's agree (rule
    `recomputed`), or its query's rows equal the message's expected rows
    (`expected`), or, weaker, its query's rows differ from trunk's
    (`changed`) or its reference side does (`changed-reference`: a fix of
    a bug in a SELECT alone, run 20261012-0317's setops-def w6-406). Not
    recomputed (the message's expected rows stand, and
    `changed-reference` compares the reference queries' rows): a compound
    compared folded (`Triage.folded`: COLLATE NOCASE or RTRIM in the query
    or on a column of a table it reads; the runner then sets rows apart
    under each column's sequence), a window item pre-triage does not
    read;
  - a query that must not fail (`the original query fails`, `a SELECT of
    the compound fails`, `the window's input fails`): trunk fails with an
    error the oracles do not skip; the patched CLI does not fail;
  - order-sorted's `rows N and N out of order`: trunk's rows are out of the
    order the query's top-level ORDER BY asks (its terms parsed: column
    numbers, or keys appended as the last columns; NULLs first ascending
    unless NULLS FIRST/LAST; SQLite's order of values); the patched CLI's
    rows are in order.
  A bug without a fix (`match` in the registry) explains a reproduced
  failure whose trunk error contains its error text (a wrong-result bug's
  is null: any reproduced failure) and whose SQL matches all its
  `sql_all` patterns and one of its `sql_any` (when it has them).
- `new`: trunk reproduces it and no known bug explains it.
- `fixed-upstream`: trunk does not reproduce it, its CLI is newer than the
  run's target (both recorded), and the target's own CLI
  (RUN_ROOT/clis/<target>/trunk) reproduces it; when that CLI is gone, the
  verdict is still fixed-upstream, with `target_cli: null` in the evidence.
- `not-reproducible`: the CLI of the run's own target check-in does not
  reproduce it (an oracle-side difference, nondeterminism, or a value the
  CLI cannot show: -0.0's sign, a NUL in text).
- `fixes-only` (a fixed run only, below): trunk does not reproduce it, the
  run's own SQLite (its target with the known bugs' fixes) does, and so
  does not the plain target when it is another check-in with a CLI: a bug
  in one of our fixes, or one the fixes expose together. Alerts like `new`.

A fixed run (SQLITE_TARGET=fixed: its SQLite is build-fixed.sh's trunk with
every open known bug's fix applied, `config.fixed_bugs` in its run.json)
is replayed on plain trunk and the single-fix CLIs as any run; when trunk
does not reproduce a finding, it is replayed on the run's own SQLite,
RUN_ROOT/clis/<target>/fixed/sqlite3 (`fixes-only`, `fixed-upstream` when
the plain target reproduces it too, `not-reproducible` when the fixed CLI
does not; without a fixed CLI, the plain rules). Known bugs whose fixes the
run has should not come up; when one does, the verdict stays `known`, with
`fix_incomplete` naming them, a `note`, and the fixed CLI's replay in the
evidence (reproducing: the fix is incomplete).
- `manual`: the message cannot be parsed or is of a kind not handled
  (`panic: ..`, a case that did not open, ..), or a replay timed out.

Each verdict goes to the example's `triage.json`, a line of the run's
`triage.jsonl`, a `triage` event in its `events.jsonl` (records.rs's JSON
lines, shipped to Datadog as logs) and the DogStatsD counter
`ephyra.sqlite.triage.verdicts` (STATSD; tags verdict, bug, oracle,
explained, profile and storage (the example's worker's loop, `gen+wal`,
which Datadog shows as `gen_wal`, and `memory` or `wal`; `unknown` and
`memory` for an example from before), and the run's tags as the runner sends
them plus TAGS); triage.json and the event carry profile and storage too;
the run's `triage.md` sums up every signature, rewritten each pass. A findings dir
given directly (no run.json above it) gets triage.jsonl, triage.md and
events.jsonl in itself.

A bug whose status is `fixed` (retired: trunk has its fix; `fixed_in`
records the check-in) is not built, matched or checked; `--check-bugs`
lists it under `retired`.

`--check-bugs` runs every open bug's check (`repro`) on the current trunk
CLI: its exact expected output means the bug is fixed upstream (event
`known_bug_fixed`, metric `ephyra.sqlite.known_bugs.fixed`), a fix that no
longer applies is `patch_stale` (event and metric
`ephyra.sqlite.known_bugs.patch_stale`), else the bug is still wrong; all in
RUN_ROOT/known-bugs-status.json.
"""

import argparse
import collections
import concurrent.futures
import functools
import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
# The case-studies directory, which the registry's paths are relative to.
SQL_DIR = os.path.dirname(os.path.dirname(HERE))
# Findings without an oracle message.
LOST = {"crashes", "hangs", "timeouts", "generator-panics"}

# The current oracles' skip lists: ephyra-sqlite src/sqlancer.rs, overflow(),
# limit() and json_error() (test_pretriage.py checks that they match).
OVERFLOW = ("integer overflow",)
LIMITS = (
    "nested too deep",
    "Expression tree is too large",
    "too many terms in compound SELECT",
    "too many columns in result set",
    "too many arguments on function",
    "too many FROM clause terms",
    "too many references to",
    "LIKE or GLOB pattern too complex",
    "string or blob too big",
    "parser stack overflow",
)
JSON_ERRORS = ("malformed JSON", "JSON cannot hold BLOB values", "bad JSON path", "json_object() labels must be TEXT")
# io_error(): the host's storage failing (the storage oracle skips it).
IO_ERRORS = ("database or disk is full", "disk I/O error")
# nondeterministic(): a case's statement refused for a date and time
# function reading the current time in one of these places.
DATE_FUNCS = ("date", "time", "datetime", "julianday", "unixepoch", "strftime", "timediff")
PURE_PLACES = ("an index", "a CHECK constraint", "a generated column")


def nondeterministic(e):
    """sqlancer.rs nondeterministic(): the message ends with
    "non-deterministic use of X() in Y", X a date and time function, Y one
    of PURE_PLACES."""
    k = e.rfind("non-deterministic use of ")
    if k < 0:
        return False
    what = e[k + len("non-deterministic use of ") :].rstrip("\n")
    return any(what == f"{f}() in {y}" for f in DATE_FUNCS for y in PURE_PLACES)

# TLP variants whose variant is three statements, joined by `;\n` and 13 spaces.
TLP_PARTS = {"tlp-where", "tlp-distinct", "tlp-group", "tlp-having"}
PART_SEP = ";\n" + " " * 13


def skip_class(e):
    """The skip an error falls in for the current oracles, or None."""
    if any(m in e for m in OVERFLOW):
        return "overflow"
    if any(m in e for m in LIMITS):
        return "limit"
    if any(m in e for m in JSON_ERRORS):
        return "json"
    if any(m in e for m in IO_ERRORS):
        return "io"
    return None


# ----------------------------------------------------------------- values
#
# A cell is a tuple: ("n",) NULL, ("i", int), ("r", float), ("t", bytes),
# ("b", bytes), or ("j", json text bytes, raw bytes) for a BLOB that is valid
# JSONB, compared as its JSON text. An outcome is ("ok", rows), ("err",
# message) or ("timeout", None).

NULL = ("n",)
MIN_I64 = -(2 ** 63)


def key(c):
    """A cell as the oracles compare it exactly: reals by their bits, JSONB
    by its text."""
    if c[0] == "r":
        return ("r", c[1].hex())
    if c[0] == "j":
        return ("j", c[1])
    return c


def equiv(c):
    """A cell as SQLite compares values: a real that is an integer in range
    is that integer (-0.0 is 0)."""
    if c[0] == "r" and c[1].is_integer() and MIN_I64 <= c[1] < 2 ** 63:
        return ("i", int(c[1]))
    return key(c)


def unsigned_zero(c):
    if c[0] == "r" and c[1] == 0.0:
        return ("r", (0.0).hex())
    return key(c)


def fmt_cell(c):
    """Rust's Debug of the cell, as in the runner's messages."""
    t = c[0]
    if t == "n":
        return "NULL"
    if t == "i":
        return str(c[1])
    if t == "r":
        x = c[1]
        if x != x:
            return "NaN"
        if x in (float("inf"), float("-inf")):
            return "inf" if x > 0 else "-inf"
        r = repr(x)
        if "e" in r:
            m, e = r.split("e")
            return f"{m.rstrip('0').rstrip('.') if '.' in m else m}e{int(e)}"
        return r
    if t == "t":
        return json.dumps(c[1].decode("utf-8", "replace"), ensure_ascii=False)
    if t == "b":
        return "x'" + c[1].hex() + "'"
    return "jsonb(" + json.dumps(c[1].decode("utf-8", "replace"), ensure_ascii=False) + ")"


def fmt_outcome(o, limit=400):
    if o is None:
        return None
    if o[0] == "ok":
        s = "Ok([" + ", ".join("[" + ", ".join(fmt_cell(c) for c in r) + "]" for r in o[1]) + "])"
    elif o[0] == "err":
        s = "Err(" + json.dumps(o[1], ensure_ascii=False) + ")"
    else:
        s = "timeout"
    return s if len(s) <= limit else s[: limit - 3] + "..."


# ------------------------------------------------------ Rust Debug parsing


class Unparsed(Exception):
    pass


_NUM = re.compile(r"-?(?:inf|NaN|\d+(?:\.\d+)?(?:e-?\d+)?)")
_ESC = {"n": "\n", "r": "\r", "t": "\t", "0": "\0", "\\": "\\", '"': '"', "'": "'"}


def _rust_string(s, i):
    if s[i] != '"':
        raise Unparsed(f"expected a string at {i}")
    i += 1
    out = []
    n = len(s)
    while i < n:
        ch = s[i]
        if ch == '"':
            return "".join(out), i + 1
        if ch == "\\":
            if i + 1 >= n:
                break
            e = s[i + 1]
            if e in _ESC:
                out.append(_ESC[e])
                i += 2
            elif e == "u" and s.startswith("{", i + 2):
                j = s.find("}", i + 3)
                if j < 0:
                    break
                out.append(chr(int(s[i + 3 : j], 16)))
                i = j + 1
            else:
                raise Unparsed(f"unknown escape \\{e}")
        else:
            out.append(ch)
            i += 1
    raise Unparsed("unterminated string")


def _cell(s, i):
    if s.startswith("NULL", i):
        return NULL, i + 4
    if s.startswith('"', i):
        t, i = _rust_string(s, i)
        return ("t", t.encode("utf-8", "surrogatepass")), i
    if s.startswith("x'", i):
        j = s.find("'", i + 2)
        if j < 0:
            raise Unparsed("unterminated blob")
        return ("b", bytes.fromhex(s[i + 2 : j])), j + 1
    if s.startswith("jsonb(", i):
        t, i = _rust_string(s, i + 6)
        if not s.startswith(")", i):
            raise Unparsed("unterminated jsonb")
        return ("j", t.encode("utf-8", "surrogatepass"), b""), i + 1
    m = _NUM.match(s, i)
    if not m:
        raise Unparsed(f"unknown value at {i}: {s[i:i + 20]!r}")
    v = m.group()
    if any(x in v for x in ".enaNi"):
        return ("r", float(v.replace("NaN", "nan"))), m.end()
    return ("i", int(v)), m.end()


def _list(s, i, item):
    if not s.startswith("[", i):
        raise Unparsed(f"expected [ at {i}")
    i += 1
    out = []
    if s.startswith("]", i):
        return out, i + 1
    while True:
        v, i = item(s, i)
        out.append(v)
        if s.startswith(", ", i):
            i += 2
        elif s.startswith("]", i):
            return out, i + 1
        else:
            raise Unparsed(f"expected , or ] at {i}")


def parse_row(s):
    """A row as Rust's Debug of Vec<Cell> (`[NULL, 1, "a"]`)."""
    row, i = _list(s, 0, _cell)
    if i != len(s):
        raise Unparsed("trailing text after a row")
    return row


def parse_outcome(s):
    """Rust's Debug of a Result<Rows, String>: `Ok([[..], ..])` or `Err("..")`."""
    s = s.strip()
    if s.startswith("Ok(") and s.endswith(")"):
        rows, i = _list(s, 3, lambda t, j: _list(t, j, _cell))
        if i != len(s) - 1:
            raise Unparsed("trailing text after the rows")
        return ("ok", rows)
    if s.startswith("Err(") and s.endswith(")"):
        msg, i = _rust_string(s, 4)
        if i != len(s) - 1:
            raise Unparsed("trailing text after the error")
        return ("err", msg)
    raise Unparsed("not an outcome: " + s[:40])


# ------------------------------------------------------- message parsing

_DIFFER = re.compile(r"^(results differ|the original fails, the variant does not|the variant fails, the original does not) \(([\w-]+)\)$")
_FAILS = (("the original query fails: ", "original"), ("a SELECT of the compound fails: ", "compound-select"), ("the window's input fails: ", "window-input"))
_ORDER = re.compile(r"^rows (\d+) and (\d+) out of order \(([\w-]+)\)$")
_LIMIT_WHAT = re.compile(r"^(.*) \(its rows (-?\d+)\.\., (-?\d+) of them\)$", re.S)
_SETOPS_WHAT = " (by definition, from its SELECTs' rows)"
# plans-noopt's last line: the optimizations off in the run the message shows
# (sqlancer.rs noopt(); the mask of SQLITE_TESTCTRL_OPTIMIZATIONS).
_OPTS = re.compile(r"^  optimizations off: (0x[0-9a-f]{8}) \((.*)\)$")
_WINDOW_WHAT = " (the windows computed from its rows)"


def noopt(variant):
    """Whether a variant is plans-noopt's: the query as written, with
    optimizations off (`plans-noopt`, or `plans-noopt-<name>` when one
    optimization alone reproduces it)."""
    return variant == "plans-noopt" or variant.startswith("plans-noopt-")


def parse_message(text):
    """The kind and the parts of a failure message (sqlancer.rs: agree(),
    original(), order(), setops(), windows()). Raises Unparsed."""
    text = text.rstrip("\n")
    lines = text.split("\n")
    head = lines[0]
    m = _DIFFER.match(head)
    opts = None
    if m and noopt(m.group(2)):
        om = _OPTS.match(lines[-1])
        if not om:
            raise Unparsed("a plans-noopt message without its optimizations line")
        opts = (int(om.group(1), 16), om.group(2))
        lines = lines[:-1]
    if m:
        if len(lines) < 5 or not lines[1].startswith("  original: ") or not lines[-1].startswith("    -> "):
            raise Unparsed("a comparison message without its two sides")
        vi = None
        for i in range(len(lines) - 2, 2, -1):
            if lines[i].startswith("  variant:  ") and lines[i - 1].startswith("    -> "):
                vi = i
                break
        if vi is None:
            raise Unparsed("no variant line")
        osql = "\n".join([lines[1][len("  original: ") :]] + lines[2 : vi - 1])
        vsql = "\n".join([lines[vi][len("  variant:  ") :]] + lines[vi + 1 : -1])
        variant = m.group(2)
        msg = {"kind": "differ", "why": m.group(1), "variant": variant, "osql": osql, "vsql": vsql,
               "orig": lines[vi - 1][len("    -> ") :], "var": lines[-1][len("    -> ") :]}
        msg["vstmts"] = vsql.split(PART_SEP) if variant in TLP_PARTS else [vsql]
        if opts:
            msg["opts"], msg["opts_names"] = opts
        lm = _LIMIT_WHAT.match(osql) if variant == "order-limit" else None
        if lm:
            msg["osql"], msg["slice"] = lm.group(1), (int(lm.group(2)), int(lm.group(3)))
        if variant == "setops-def" and osql.endswith(_SETOPS_WHAT):
            msg["expected_side"] = "orig"
        if variant == "window-def" and vsql.endswith(_WINDOW_WHAT):
            msg["expected_side"] = "var"
        return msg
    for prefix, what in _FAILS:
        if text.startswith(prefix):
            k = text.rfind("\n  -> ")
            if k < 0:
                raise Unparsed("no error line")
            return {"kind": "fails", "what": what, "sql": text[len(prefix) : k], "error": text[k + len("\n  -> ") :]}
    m = _ORDER.match(head)
    if m:
        if len(lines) < 4 or not lines[1].startswith("  query: ") or not all(l.startswith("    -> ") for l in lines[-2:]):
            raise Unparsed("an order message without its query and rows")
        sql = "\n".join([lines[1][len("  query: ") :]] + lines[2:-2])
        return {"kind": "order", "variant": m.group(3), "k": int(m.group(1)), "sql": sql,
                "rows": [lines[-2][len("    -> ") :], lines[-1][len("    -> ") :]]}
    raise Unparsed("a message of a kind pre-triage does not handle: " + head[:100])


# ------------------------------------------------------------ SQL shapes

_TOKEN = re.compile(
    r"""(?P<ws>\s+)
      |(?P<str>[xX]?'(?:[^']|'')*')
      |(?P<qid>"(?:[^"]|"")*"|`(?:[^`]|``)*`|\[[^\]]*\])
      |(?P<com>--[^\n]*|/\*.*?\*/)
      |(?P<num>\d+(?:\.\d*)?(?:[eE][+-]?\d+)?|\.\d+(?:[eE][+-]?\d+)?)
      |(?P<word>[^\W\d]\w*)
      |(?P<op>\|\||<<|>>|<=|>=|==|!=|<>|->>|->|.)""",
    re.X | re.S,
)


def scan(sql):
    """The SQL's tokens as (kind, text, depth, start, end): depth the
    parentheses around the token (a parenthesis has the depth outside it),
    start and end its place in the SQL."""
    out = []
    depth = 0
    for m in _TOKEN.finditer(sql):
        kind = m.lastgroup
        if kind in ("ws", "com"):
            continue
        t = m.group()
        if t == ")":
            depth -= 1
        out.append((kind, t, depth, m.start(), m.end()))
        if t == "(":
            depth += 1
    return out


def _is(tok, *words):
    return tok[0] == "word" and tok[1].upper() in words


class Shape:
    """A query's top level: its SELECTs (token lists), the compound
    operators between them, and its ORDER BY terms (None without one), each
    (("col", index) or ("expr", text), desc, nulls_first)."""

    def __init__(self, sql):
        toks = scan(sql)
        start = next((i for i, t in enumerate(toks) if t[2] == 0 and _is(t, "SELECT", "VALUES")), len(toks))
        self.cores, self.ops, self.order = [], [], None
        cur = []
        i = start
        while i < len(toks):
            t = toks[i]
            if t[2] == 0 and _is(t, "UNION", "INTERSECT", "EXCEPT"):
                op = t[1].upper()
                if op == "UNION" and i + 1 < len(toks) and _is(toks[i + 1], "ALL"):
                    op, i = "UNION ALL", i + 1
                self.cores.append(cur)
                self.ops.append(op)
                cur = []
            elif t[2] == 0 and _is(t, "ORDER") and i + 1 < len(toks) and _is(toks[i + 1], "BY"):
                end = next((j for j in range(i + 2, len(toks)) if toks[j][2] == 0 and _is(toks[j], "LIMIT")), len(toks))
                self.order = self._terms(toks[i + 2 : end])
                i = end
                continue
            elif t[2] == 0 and _is(t, "LIMIT"):
                break
            else:
                cur.append(t)
            i += 1
        self.cores.append(cur)

    @staticmethod
    def _terms(toks):
        terms, cur = [], []
        for t in toks + [("op", ",", 0)]:
            if t[2] == 0 and t[1] == ",":
                if not cur:
                    continue
                nulls = None
                if len(cur) >= 2 and _is(cur[-2], "NULLS") and _is(cur[-1], "FIRST", "LAST"):
                    nulls = cur[-1][1].upper() == "FIRST"
                    cur = cur[:-2]
                desc = False
                if cur and _is(cur[-1], "ASC", "DESC"):
                    desc = cur[-1][1].upper() == "DESC"
                    cur = cur[:-1]
                if len(cur) == 1 and cur[0][0] == "num" and cur[0][1].isdigit():
                    k = ("col", int(cur[0][1]) - 1)
                else:
                    k = ("expr", " ".join(x[1] for x in cur))
                terms.append((k, desc, (not desc) if nulls is None else nulls))
                cur = []
            else:
                cur.append(t)
        return terms


def _single_agg(items):
    if len(items) >= 3 and items[0][0] == "word" and items[0][1].lower() in ("count", "sum", "min", "max") and items[1][1] == "(" and items[-1][1] == ")":
        d = items[0][2]
        if items[-1][2] == d and all(t[2] > d for t in items[2:-1]):
            return items[0][1].lower()
    return None


def core_mode(core):
    """sqlancer.rs unordered_mode() of one SELECT: one aggregate (scalar, or
    sum), DISTINCT (set), GROUP BY (groups), else multiset."""
    if not core or not _is(core[0], "SELECT"):
        return "multiset"
    j = 1
    distinct = j < len(core) and _is(core[j], "DISTINCT")
    if j < len(core) and _is(core[j], "DISTINCT", "ALL"):
        j += 1
    k = next((x for x in range(j, len(core)) if core[x][2] == 0 and _is(core[x], "FROM")), len(core))
    grouped = any(t[2] == 0 and _is(t, "GROUP") for t in core[k:])
    agg = _single_agg(core[j:k])
    if agg and not grouped:
        return "sum" if agg == "sum" else "scalar"
    if distinct:
        return "set"
    if grouped:
        return "groups"
    return "multiset"


def unordered_mode(sql):
    s = Shape(sql)
    if s.ops:
        exact = all(op == "UNION ALL" for op in s.ops) and all(core_mode(c) == "multiset" for c in s.cores)
        return "multiset" if exact else "equiv"
    return core_mode(s.cores[0])


def query_mode(sql):
    """sqlancer.rs mode(): in order with a top-level ORDER BY (every ORDER BY
    the generator writes is total), else unordered_mode."""
    return "ordered" if Shape(sql).order is not None else unordered_mode(sql)


def infer_mode(variant, osql, vsql):
    """How the oracle compared the two sides of a comparison, from the
    variant's name and the SQL (sqlancer.rs: norec_core, tlp_core, plans,
    order, setops, ctes, windows)."""
    if variant in ("norec", "norec-join"):
        return "scalar"
    if variant == "tlp-agg":
        return "sum" if unordered_mode(osql) == "sum" else "scalar"
    if variant == "tlp-where":
        return "multiset"
    if variant in ("tlp-distinct", "tlp-group", "tlp-intersect", "tlp-except"):
        return "set"
    if variant == "tlp-having":
        return unordered_mode(osql)
    if variant == "tlp-union":
        ops = Shape(vsql).ops
        if any(op != "UNION ALL" for op in ops):
            return "set"
        return unordered_mode(osql)
    if variant in ("order-limit", "order-sorted"):
        return "ordered"
    if variant == "order-multiset":
        return unordered_mode(vsql)
    if variant == "plans-full-union":
        return unordered_mode(osql)
    if variant == "window-def":
        return "multiset"
    if variant == "setops-def":
        return query_mode(vsql)
    return query_mode(osql)


# ------------------------------------------------------ reference sides
#
# setops-def and window-def compare a query with rows the runner computed
# in Rust from other queries' rows (sqlancer.rs setops(), windows()). The
# same computation here, on each CLI's rows of those queries, recomputes
# that side with the CLI, as every other comparison's two sides are run.


def _num(toks, i):
    """A signed integer literal at toks[i]: (value, next index)."""
    sign = 1
    if i < len(toks) and toks[i][1] in ("-", "+"):
        sign, i = (-1 if toks[i][1] == "-" else 1), i + 1
    if i >= len(toks) or toks[i][0] != "num" or not toks[i][1].isdigit():
        raise Unparsed("a LIMIT that is not integers")
    return sign * int(toks[i][1]), i + 1


def compound_parts(sql):
    """A compound query as setops() runs it: each SELECT alone as its own
    query (Query::cores: the WITH clause, then the SELECT, without the
    compound's ORDER BY and LIMIT), the operators, the ORDER BY terms that
    are result columns ((index, desc, nulls_first)) and the LIMIT ((count,
    offset), or None). Raises Unparsed."""
    toks = scan(sql)
    start = next((i for i, t in enumerate(toks) if t[2] == 0 and _is(t, "SELECT")), None)
    if start is None:
        raise Unparsed("no top-level SELECT")
    with_sql = sql[: toks[start][3]]
    cores, ops, cur, limit, end = [], [], [], None, len(toks)
    i = start
    while i < len(toks):
        t = toks[i]
        if t[2] == 0 and _is(t, "UNION", "INTERSECT", "EXCEPT"):
            op = t[1].upper()
            if op == "UNION" and i + 1 < len(toks) and _is(toks[i + 1], "ALL"):
                op, i = "UNION ALL", i + 1
            cores.append(cur)
            ops.append(op)
            cur = []
        elif t[2] == 0 and (_is(t, "ORDER") or _is(t, "LIMIT")):
            end = i
            break
        else:
            cur.append(t)
        i += 1
    cores.append(cur)
    if not ops or any(not c for c in cores):
        raise Unparsed("not a compound")
    k = next((j for j in range(end, len(toks)) if toks[j][2] == 0 and _is(toks[j], "LIMIT")), None)
    if k is not None:
        a, j = _num(toks, k + 1)
        if j < len(toks) and _is(toks[j], "OFFSET"):
            b, j = _num(toks, j + 1)
            limit = (a, b)
        elif j < len(toks) and toks[j][1] == ",":
            b, j = _num(toks, j + 1)
            limit = (b, a)
        else:
            limit = (a, 0)
        if j != len(toks):
            raise Unparsed("text after the LIMIT")
    order = [(key[1], desc, nf) for (key, desc, nf) in (Shape(sql).order or []) if key[0] == "col"]
    return {"cores": [with_sql + sql[c[0][3] : c[-1][4]] for c in cores], "ops": ops, "order": order, "limit": limit}


def equiv_cell(c):
    """sqlancer.rs Cell::equiv: a real that is an integer in range is that
    integer (-0.0 is 0)."""
    if c[0] == "r" and c[1].is_integer() and MIN_I64 <= c[1] < 2 ** 63:
        return ("i", int(c[1]))
    return c


def bytes_key(row):
    """A row as SQLite tells rows apart in a set: values exactly, a JSONB
    value by its bytes (sqlancer.rs set_op's bytes())."""
    return tuple(("b", c[2]) if c[0] == "j" else (("r", c[1].hex()) if c[0] == "r" else c) for c in row)


def set_op(op, left, right):
    """sqlancer.rs set_op(): left op right by the definition of op."""
    def distinct(rows):
        seen, out = set(), []
        for r in rows:
            k = bytes_key(r)
            if k not in seen:
                seen.add(k)
                out.append(r)
        return out
    if op == "UNION ALL":
        return left + right
    if op == "UNION":
        return distinct(left + right)
    keys = {bytes_key(r) for r in right}
    return [r for r in distinct(left) if (bytes_key(r) in keys) == (op == "INTERSECT")]


def cmp_rows(a, b, terms):
    """sqlancer.rs cmp_rows(): two rows' order under terms (column, desc,
    nulls_first)."""
    for (c, desc, nulls_first) in terms:
        x, y = a[c], b[c]
        if x == NULL and y == NULL:
            o = 0
        elif x == NULL:
            o = -1 if nulls_first else 1
        elif y == NULL:
            o = 1 if nulls_first else -1
        else:
            o = cmp_cells(x, y)
            if desc:
                o = -o
        if o:
            return o
    return 0


def setops_reference(parts, outcomes, equiv):
    """The compound computed from its SELECTs' outcomes (sqlancer.rs
    setops(): left to right, on rows as SQLite compares values when equiv,
    then sorted by the result-column terms and cut by the LIMIT), or the
    first SELECT's error."""
    for o in outcomes:
        if o[0] != "ok":
            return o
    rows = [[[equiv_cell(c) for c in r] for r in o[1]] if equiv else o[1] for o in outcomes]
    out = rows[0]
    for op, r in zip(parts["ops"], rows[1:]):
        out = set_op(op, out, r)
    if parts["order"]:
        out = sorted(out, key=functools.cmp_to_key(lambda a, b: cmp_rows(a, b, parts["order"])))
        if parts["limit"]:
            count, offset = parts["limit"]
            start = min(max(offset, 0), len(out))
            out = out[start:] if count < 0 else out[start : start + count]
    return ("ok", out)


_WIN_FUNCS = ("row_number", "rank", "dense_rank", "count", "sum", "min", "max")


def _items(sql):
    """The top-level SELECT's items (token lists) of a query that is not a
    compound."""
    toks = scan(sql)
    start = next((i for i, t in enumerate(toks) if t[2] == 0 and _is(t, "SELECT")), None)
    if start is None:
        raise Unparsed("no top-level SELECT")
    j = start + 1
    if j < len(toks) and _is(toks[j], "DISTINCT", "ALL"):
        j += 1
    k = next((x for x in range(j, len(toks)) if toks[x][2] == 0 and _is(toks[x], "FROM")), len(toks))
    items, cur = [], []
    for t in toks[j:k]:
        if t[2] == 0 and t[1] == ",":
            items.append(cur)
            cur = []
        else:
            cur.append(t)
    items.append(cur)
    return items


def _split(toks, depth):
    """Tokens split at the commas of a depth."""
    out, cur = [], []
    for t in toks:
        if t[2] == depth and t[1] == ",":
            out.append(cur)
            cur = []
        else:
            cur.append(t)
    if cur:
        out.append(cur)
    return out


def _bound(toks, i):
    """A frame bound at toks[i] (expr.rs Bound::sql): (bound, next index)."""
    w = lambda k, *ws: k < len(toks) and _is(toks[k], *ws)
    if w(i, "UNBOUNDED") and w(i + 1, "PRECEDING"):
        return ("up",), i + 2
    if w(i, "UNBOUNDED") and w(i + 1, "FOLLOWING"):
        return ("uf",), i + 2
    if w(i, "CURRENT") and w(i + 1, "ROW"):
        return ("cur",), i + 2
    if i < len(toks) and toks[i][0] == "num" and toks[i][1].isdigit() and w(i + 1, "PRECEDING", "FOLLOWING"):
        return ("p" if toks[i + 1][1].upper() == "PRECEDING" else "f", int(toks[i][1])), i + 2
    raise Unparsed("a frame bound pre-triage does not read")


def window_item(item):
    """A window item as expr.rs renders one (`func([arg | *]) OVER
    ([PARTITION BY e, ..] [ORDER BY e [DESC] [NULLS ..], ..] [frame])`):
    its function, whether it has an argument, its PARTITION BY term count,
    its ORDER BY terms ((desc, nulls_first)) and frame ((mode, start, end,
    exclude), or None); None for any other item. Raises Unparsed."""
    if len(item) < 6 or item[0][0] != "word" or item[1][1] != "(":
        return None
    d = item[0][2]
    close = next((k for k in range(2, len(item)) if item[k][1] == ")" and item[k][2] == d), None)
    if close is None or close + 2 >= len(item) or not _is(item[close + 1], "OVER") or item[close + 2][1] != "(" or item[-1][1] != ")" or item[-1][2] != d:
        return None
    func = item[0][1].lower()
    if func not in _WIN_FUNCS:
        raise Unparsed(f"a window function pre-triage does not compute: {func}")
    args = item[2:close]
    arg = bool(args) and not (len(args) == 1 and args[0][1] == "*")
    over = item[close + 3 : -1]
    top = d + 1
    at = lambda k, *ws: k < len(over) and over[k][2] == top and _is(over[k], *ws)
    i, np, order, frame = 0, 0, [], None
    def until(i, *stops):
        j = i
        while j < len(over) and not at(j, *stops):
            j += 1
        return j
    if at(0, "PARTITION") and at(1, "BY"):
        j = until(2, "ORDER", "ROWS", "RANGE", "GROUPS")
        np, i = len(_split(over[2:j], top)), j
    if at(i, "ORDER") and at(i + 1, "BY"):
        j = until(i + 2, "ROWS", "RANGE", "GROUPS")
        for term in _split(over[i + 2 : j], top):
            nulls = None
            if len(term) >= 2 and _is(term[-2], "NULLS") and _is(term[-1], "FIRST", "LAST"):
                nulls, term = term[-1][1].upper() == "FIRST", term[:-2]
            desc = bool(term) and _is(term[-1], "DESC")
            order.append((desc, (not desc) if nulls is None else nulls))
        i = j
    if at(i, "ROWS", "RANGE", "GROUPS"):
        mode = over[i][1].upper()
        if at(i + 1, "BETWEEN"):
            start, j = _bound(over, i + 2)
            if not at(j, "AND"):
                raise Unparsed("a frame without AND")
            end, j = _bound(over, j + 1)
        else:
            start, j = _bound(over, i + 1)
            end = ("cur",)
        exclude = "no others"
        if at(j, "EXCLUDE"):
            words = " ".join(t[1].lower() for t in over[j + 1 :])
            if words not in ("no others", "current row", "group", "ties"):
                raise Unparsed("an EXCLUDE pre-triage does not read")
            exclude, j = words, len(over)
        frame, i = (mode, start, end, exclude), j
    if i != len(over):
        raise Unparsed("a window definition pre-triage does not read")
    return {"func": func, "arg": arg, "np": np, "order": order, "frame": frame}


def window_values(w, rows):
    """sqlancer.rs window_values(): the values of window w over rows
    (partition values, order values, argument or None), in their order; or
    an error outcome."""
    parts = collections.OrderedDict()
    for i, r in enumerate(rows):
        parts.setdefault(tuple(bytes_key([equiv_cell(c) for c in r[0]])), []).append(i)
    terms = [(k, desc, nf) for k, (desc, nf) in enumerate(w["order"])]
    out = [NULL] * len(rows)
    for idx in parts.values():
        idx = sorted(idx, key=functools.cmp_to_key(lambda a, b: cmp_rows(rows[a][1], rows[b][1], terms)))
        n = len(idx)
        group, starts = [0] * n, [0]
        for p in range(1, n):
            new = cmp_rows(rows[idx[p - 1]][1], rows[idx[p]][1], terms) != 0
            group[p] = group[p - 1] + int(new)
            if new:
                starts.append(p)
        groups = len(starts)
        first = lambda g: 0 if g < 0 else (n if g >= groups else starts[g])
        last = lambda g: -1 if g < 0 else (n - 1 if g + 1 >= groups else starts[g + 1] - 1)
        for p in range(n):
            g = group[p]
            f = w["func"]
            if f == "row_number":
                value = ("i", p + 1)
            elif f == "rank":
                value = ("i", starts[g] + 1)
            elif f == "dense_rank":
                value = ("i", g + 1)
            else:
                fr = w["frame"]
                if fr is None:
                    lo, hi = (0, n - 1) if not w["order"] else (0, last(g))
                else:
                    mode = fr[0]

                    def at(b, start):
                        if b[0] == "up":
                            return 0
                        if b[0] == "uf":
                            return n - 1
                        if mode == "ROWS":
                            return p - b[1] if b[0] == "p" else (p if b[0] == "cur" else p + b[1])
                        if b[0] == "p":
                            return first(g - b[1]) if start else last(g - b[1])
                        if b[0] == "cur":
                            return first(g) if start else last(g)
                        return first(g + b[1]) if start else last(min(g + b[1], groups - 1))

                    lo, hi = at(fr[1], True), at(fr[2], False)
                ex = fr[3] if fr else "no others"
                frame = [q for q in range(max(lo, 0), min(hi, n - 1) + 1)
                         if ex == "no others" or (ex == "current row" and q != p) or (ex == "group" and group[q] != group[p])
                         or (ex == "ties" and (q == p or group[q] != group[p]))]
                args = [rows[idx[q]][2] for q in frame]
                values = [c for c in args if c is not None and c != NULL]
                if f == "count":
                    value = ("i", len(args) if not w["arg"] else len(values))
                elif f == "sum":
                    if not values:
                        value = NULL
                    else:
                        if any(c[0] != "i" for c in values):
                            bad = next(c for c in values if c[0] != "i")
                            return ("err", f"a sum of a value that is not an integer: {fmt_cell(bad)}")
                        total = sum(c[1] for c in values)
                        if not MIN_I64 <= total < 2 ** 63:
                            return ("err", "integer overflow (computed)")
                        value = ("i", total)
                elif not values:
                    value = NULL
                elif f == "min":
                    value = values[0]
                    for c in values[1:]:
                        if cmp_cells(c, value) < 0:
                            value = c
                else:
                    value = values[0]
                    for c in values[1:]:
                        if cmp_cells(c, value) >= 0:
                            value = c
            out[idx[p]] = value
    return ("ok", out)


def windows_reference(windows, width, base):
    """The window query's rows computed from its input's outcome (sqlancer.rs
    windows(): the items, each window's value in its place), or the
    input's error; None when the input's columns are not the windows'."""
    if base[0] != "ok":
        return base
    need = width + sum(w["np"] + len(w["order"]) + int(w["arg"]) for w in windows if w)
    if any(len(r) != need for r in base[1]):
        return None
    rows = [list(r[:width]) for r in base[1]]
    at = width
    for k, w in enumerate(windows):
        if not w:
            continue
        np, nk = w["np"], len(w["order"])
        inp = [(r[at : at + np], r[at + np : at + np + nk], r[at + np + nk] if w["arg"] else None) for r in base[1]]
        at += np + nk + int(w["arg"])
        v = window_values(w, inp)
        if v[0] != "ok":
            return v
        for row, x in zip(rows, v[1]):
            row[k] = x
    return ("ok", rows)


# ---------------------------------------------------------- comparisons


def compare_rows(mode, a, b):
    """Whether rows a and b agree under mode (sqlancer.rs agree()); ordered
    compares values as SQLite does (Ordered(false), the looser: whether the
    order was by every source's key is not known here)."""
    if mode == "multiset":
        return collections.Counter(tuple(map(key, r)) for r in a) == collections.Counter(tuple(map(key, r)) for r in b)
    if mode == "groups":
        return collections.Counter(tuple(map(unsigned_zero, r)) for r in a) == collections.Counter(tuple(map(unsigned_zero, r)) for r in b)
    if mode == "set":
        return {tuple(map(equiv, r)) for r in a} == {tuple(map(equiv, r)) for r in b}
    if mode == "equiv":
        return collections.Counter(tuple(map(equiv, r)) for r in a) == collections.Counter(tuple(map(equiv, r)) for r in b)
    if mode == "ordered":
        return [tuple(map(equiv, r)) for r in a] == [tuple(map(equiv, r)) for r in b]
    one = lambda rows: rows[0][0] if len(rows) == 1 and len(rows[0]) == 1 else None
    x, y = one(a), one(b)
    if x is None or y is None:
        return False
    if mode == "scalar":
        return equiv(x) == equiv(y)
    if mode == "sum":
        # Real sums are not compared; a NULL sum against a real may be a NaN
        # (the oracle asks count(e), which is not known here).
        if x[0] == "r" and y[0] == "r":
            return True
        if (x == NULL and y[0] == "r") or (y == NULL and x[0] == "r"):
            return True
        return key(x) == key(y)
    raise ValueError(mode)


def rows_only(a, b):
    """The rows of a that b lacks (as multisets, values exactly)."""
    d = collections.Counter(tuple(map(key, r)) for r in a) - collections.Counter(tuple(map(key, r)) for r in b)
    out = []
    for r in a:
        k = tuple(map(key, r))
        if d[k] > 0:
            d[k] -= 1
            out.append(r)
    return out


def agree(mode, o, v):
    """(agree, why) for outcomes o and v under mode: both failing agree, one
    failing with an error the oracles skip agrees."""
    if o[0] == "timeout" or v[0] == "timeout":
        return None, "timeout"
    if o[0] == "err" and v[0] == "err":
        return True, "both fail"
    if o[0] == "err" or v[0] == "err":
        e = o[1] if o[0] == "err" else v[1]
        c = skip_class(e)
        if c:
            return True, f"one side skipped ({c})"
        return False, ("the original fails" if o[0] == "err" else "the variant fails")
    same = compare_rows(mode, o[1], v[1])
    return same, ("agree" if same else "results differ")


def cmp_cells(a, b):
    """SQLite's order of two values (BINARY, UTF-8): NULL, numbers, text,
    blobs (JSONB by its bytes)."""
    rank = {"n": 0, "i": 1, "r": 1, "t": 2, "b": 3, "j": 3}
    ra, rb = rank[a[0]], rank[b[0]]
    if ra != rb:
        return -1 if ra < rb else 1
    if ra == 0:
        return 0
    if ra == 1:
        x, y = a[1], b[1]
    elif ra == 2:
        x, y = a[1], b[1]
    else:
        x = a[2] if a[0] == "j" else a[1]
        y = b[2] if b[0] == "j" else b[1]
    return (x > y) - (x < y)


def out_of_order(rows, terms):
    """The first k with rows k and k+1 out of order under the ORDER BY terms,
    or None; terms' extra keys are the last columns."""
    if not rows:
        return None
    extra = sum(1 for t in terms if t[0][0] == "expr")
    width = len(rows[0]) - extra
    cols, e = [], 0
    for (k, desc, nulls_first) in terms:
        if k[0] == "col":
            cols.append((k[1], desc, nulls_first))
        else:
            cols.append((width + e, desc, nulls_first))
            e += 1
    for n in range(len(rows) - 1):
        a, b = rows[n], rows[n + 1]
        for (c, desc, nulls_first) in cols:
            if c >= len(a) or c >= len(b):
                return None
            x, y = a[c], b[c]
            if x == NULL and y == NULL:
                o = 0
            elif x == NULL:
                o = -1 if nulls_first else 1
            elif y == NULL:
                o = 1 if nulls_first else -1
            else:
                o = cmp_cells(x, y)
                if desc:
                    o = -o
            if o:
                if o > 0:
                    return n
                break
    return None


# ------------------------------------------------------------- the CLIs

_CLI_ERROR = re.compile(r"^(?:Parse error|Runtime error|Error)(?: near line (\d+))?: (?:in prepare, |stepping, )?(.*)$")


def cli_error(stderr, from_line=None):
    """The error the sqlite3 CLI printed, without a trailing result code;
    with `from_line`, the first one of a statement from that line of the
    script on (a replayed case's statements come before the query)."""
    text = stderr.decode("utf-8", "replace")
    for line in text.splitlines():
        m = _CLI_ERROR.match(line)
        if m:
            if from_line is not None and m.group(1) is not None and int(m.group(1)) < from_line:
                continue
            return re.sub(r" \(\d+\)$", "", m.group(2)).strip()
        if from_line is not None:
            continue
    if from_line is not None:
        return None
    text = text.strip()
    return text.splitlines()[0] if text else None


_QNUM = re.compile(rb"-?(?:Inf|\d+(?:\.\d+)?(?:e[+-]?\d+)?)")


def _qtext(line, i):
    """A quoted text at line[i] (`'..'` with '' escapes), its bytes."""
    out = bytearray()
    i += 1
    while i < len(line):
        if line[i] == 0x27:
            if line[i + 1 : i + 2] == b"'":
                out += b"'"
                i += 2
                continue
            return bytes(out), i + 1
        out.append(line[i])
        i += 1
    raise Unparsed("unterminated text in the CLI's output")


def _unistr(raw):
    out = bytearray()
    i = 0
    while i < len(raw):
        if raw[i] == 0x5C:
            if raw[i + 1 : i + 2] == b"\\":
                out += b"\\"
                i += 2
                continue
            if raw[i + 1 : i + 2] in (b"u", b"U"):
                n = 4 if raw[i + 1 : i + 2] == b"u" else 8
                if raw[i + 2 : i + 3] == b"+":
                    n, i = 6, i + 1
                out += chr(int(raw[i + 2 : i + 2 + n], 16)).encode("utf-8", "surrogatepass")
                i += 2 + n
                continue
        out.append(raw[i])
        i += 1
    return bytes(out)


def _qcell(line, i):
    if line.startswith(b"NULL", i):
        return NULL, i + 4
    if line[i : i + 2] in (b"x'", b"X'"):
        j = line.index(b"'", i + 2)
        return ("b", bytes.fromhex(line[i + 2 : j].decode())), j + 1
    if line.startswith(b"'", i) or line.startswith(b"unistr('", i) or line.startswith(b"char(", i):
        out = bytearray()
        while True:
            if line.startswith(b"'", i):
                t, i = _qtext(line, i)
                out += t
            elif line.startswith(b"unistr('", i):
                t, i = _qtext(line, i + 7)
                if not line.startswith(b")", i):
                    raise Unparsed("unterminated unistr()")
                out += _unistr(t)
                i += 1
            elif line.startswith(b"char(", i):
                j = line.index(b")", i)
                out += "".join(chr(int(x)) for x in line[i + 5 : j].split(b",")).encode("utf-8", "surrogatepass")
                i = j + 1
            else:
                raise Unparsed("unknown text part in the CLI's output")
            if line.startswith(b"||", i):
                i += 2
                continue
            return ("t", bytes(out)), i
    m = _QNUM.match(line, i)
    if not m:
        raise Unparsed(f"unknown value in the CLI's output: {line[i:i + 20]!r}")
    v = m.group()
    if v in (b"Inf", b"-Inf"):
        return ("r", float(v.decode().replace("Inf", "inf"))), m.end()
    if b"." in v or b"e" in v:
        return ("r", float(v)), m.end()
    return ("i", int(v)), m.end()


def parse_quote(out):
    """The rows of the CLI's `.mode quote` output (a row a line: text with a
    newline is written unistr(..))."""
    rows = []
    for line in out.split(b"\n"):
        if line.endswith(b"\r"):
            line = line[:-1]
        if not line:
            continue
        row, i = [], 0
        while True:
            c, i = _qcell(line, i)
            row.append(c)
            if i == len(line):
                break
            if line[i : i + 1] != b",":
                raise Unparsed(f"expected , in the CLI's output at {i}")
            i += 1
        rows.append(row)
    return rows


class Cli:
    """A CLI. With `reindex`, every copy of a database it replays on is
    rebuilt by its own `REINDEX` first: a bug whose fix is in the code that
    writes index entries (the registry's `reindex: true`) is in after.db's
    indexes already, written by the run's unpatched SQLite, and only a
    rebuild with the patched CLI shows what the fix changes."""

    def __init__(self, name, path, checkin, reindex=False):
        self.name, self.path, self.checkin, self.reindex = name, path, checkin, reindex


# Outcomes with more rows are not cached; past BIG_ROWS in an example, the
# patched CLIs run one at a time.
CACHE_ROWS = 10000
BIG_ROWS = 20000


# How a statement is replayed (the example's storage, Triage.mode): on a
# plain copy of after.db, on a copy in WAL mode, or in memory (the CLI's
# `.open --deserialize` of a copy).
MODES = ("file", "wal", "memory")


class Replayer:
    """Runs statements with the CLIs, each CLI on its own copy of the
    example's database (one per mode), one process a statement; caches
    outcomes by CLI, database bytes, SQL and mode."""

    def __init__(self, timeout):
        self.timeout = timeout
        self.cache = {}
        self.tmp = tempfile.mkdtemp(prefix="pretriage-")
        self.copies = {}
        self.serial = 0
        self.rows_seen = 0
        self.lock = threading.Lock()
        # The current example's connection settings (connection_pragmas).
        self.prelude = b""
        # The current example's case statements, when its after.db is not
        # its connection (set_case), else None.
        self.case = None

    def close(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _db(self, cli, db, mode="file"):
        """The CLI's own copy of the database for a mode (the patched CLIs
        run side by side, each in a thread): for `wal`, put in WAL mode
        first."""
        k = (cli.path, cli.reindex, db, mode)
        with self.lock:
            if k not in self.copies:
                self.serial += 1
                p = os.path.join(self.tmp, f"{cli.name}-{self.serial}-{mode}.sqlite")
                if db:
                    shutil.copyfile(db, p)
                    if cli.reindex:
                        self._exec(cli, p, b"REINDEX;\n")
                if mode == "wal":
                    self._exec(cli, p, b"PRAGMA journal_mode=WAL;\n")
                self.copies[k] = p
            return self.copies[k]

    def new_example(self):
        self.rows_seen = 0
        self.prelude = b""
        self.case = None
        for p in self.copies.values():
            for f in (p, p + "-journal", p + "-wal", p + "-shm"):
                try:
                    os.unlink(f)
                except OSError:
                    pass
        self.copies = {}

    def set_case(self, script, before=None, readonly=False, wal=None):
        """Replay every statement after the case's statements (`script`,
        their output discarded, their errors not the statement's): on
        `before`, the case's starting database (the runner's case.file),
        as the runner opened it (in memory, with a WAL header's version
        bytes made rollback ones; a WAL loop's on a fresh file copy, put in
        WAL mode with the case's `wal` settings unless it is already), or
        without one on the after.db copies, main read-only with
        `readonly`."""
        fixed = None
        if before:
            with open(before, "rb") as f:
                data = bytearray(f.read())
            for at in (18, 19):
                if len(data) > at and data[at] == 2:
                    data[at] = 1
            self.serial += 1
            fixed = os.path.join(self.tmp, f"before-{self.serial}.sqlite")
            with open(fixed, "wb") as f:
                f.write(bytes(data))
        self.case = {"script": script, "before": before, "before_memory": fixed, "readonly": readonly, "wal": wal or {}}

    def _case_run(self, cli, db, mode, tail):
        """A statement's run after the case's statements (set_case): the
        CLI's process, and the script's line where `tail` starts."""
        c = self.case
        head, dbpath, scratch = b"", None, None
        if c["before"]:
            if mode == "wal":
                self.serial += 1
                scratch = os.path.join(self.tmp, f"{cli.name}-{self.serial}-case.sqlite")
                shutil.copyfile(c["before"], scratch)
                if cli.reindex:
                    self._exec(cli, scratch, b"REINDEX;\n")
                w = c["wal"]
                head = (f"PRAGMA cache_size = {int(w.get('cache_size', -2000))};\nPRAGMA synchronous = {w.get('synchronous', 'FULL')};\n"
                        f"PRAGMA wal_autocheckpoint = {int(w.get('autocheckpoint', 1000))};\n").encode()
                with open(c["before"], "rb") as f:
                    hdr = f.read(20)
                if len(hdr) < 20 or hdr[19] != 2:
                    head += b"PRAGMA journal_mode = WAL;\n"
                dbpath = scratch
            else:
                path = self._db(cli, c["before_memory"], "memory").replace('"', '\\"')
                head, dbpath = f'.open --deserialize "{path}"\n'.encode(), ":memory:"
        else:
            if mode == "memory" and not c["readonly"]:
                path = self._db(cli, db, mode).replace('"', '\\"')
                head, dbpath = f'.open --deserialize "{path}"\n'.encode(), ":memory:"
            elif c["readonly"]:
                # Main alone read-only (a URI's mode=ro; the CLI's -readonly
                # would make an ATTACHed database so too), on the plain
                # copy: a read-only WAL file needs its -shm, which not
                # every CLI creates read-only.
                dbpath = "file:" + urllib.parse.quote(os.path.abspath(self._db(cli, db, "file"))) + "?mode=ro"
            else:
                dbpath = self._db(cli, db, "wal" if mode == "wal" else "file")
        script = b".output /dev/null\n" + head + c["script"] + b"\n;\n.output\n"
        line = script.count(b"\n") + 1
        try:
            return self._exec(cli, dbpath, script + tail), line
        finally:
            if scratch:
                for f in (scratch, scratch + "-journal", scratch + "-wal", scratch + "-shm"):
                    try:
                        os.unlink(f)
                    except OSError:
                        pass

    def _exec(self, cli, dbpath, script):
        """Run the CLI on a script; its output goes to files, so that a CLI
        writing many rows never waits on this process (the timeout is the
        CLI's own time)."""
        with tempfile.TemporaryFile(dir=self.tmp) as out, tempfile.TemporaryFile(dir=self.tmp) as err:
            try:
                p = subprocess.run([cli.path, "-batch", "-init", os.devnull, dbpath], input=script,
                                   stdout=out, stderr=err, timeout=self.timeout, cwd=self.tmp)
            except subprocess.TimeoutExpired:
                return None
            out.seek(0)
            err.seek(0)
            p.stdout, p.stderr = out.read(), err.read()
        return p

    def new_signature(self):
        """Examples of one signature are often the same query on the same
        database: outcomes are cached within a signature only."""
        self.cache.clear()

    def run(self, cli, db, db_digest, sql, mode="file", opts=None):
        """One statement's outcome; with `opts`, a mask of
        SQLITE_TESTCTRL_OPTIMIZATIONS, those optimizations off for it (the
        CLI's `.testctrl optimizations MASK`, its output discarded)."""
        if mode not in MODES:
            raise ValueError(mode)
        case = self.case and (self.case["script"], self.case["before"], self.case["readonly"])
        k = (cli.path, cli.reindex, db_digest, sql, mode, self.prelude, opts, case)
        if k in self.cache:
            return self.cache[k]
        off = b"" if opts is None else f".output /dev/null\n.testctrl optimizations {opts:#010x}\n.output\n".encode()
        script = self.prelude + off + b".mode quote\n" + sql.encode("utf-8", "surrogateescape") + b";\n"
        line = None
        if self.case is not None:
            p, line = self._case_run(cli, db, mode, script)
        elif mode == "memory":
            path = self._db(cli, db, mode).replace('"', '\\"')
            p = self._exec(cli, ":memory:", f'.open --deserialize "{path}"\n'.encode() + script)
        else:
            p = self._exec(cli, self._db(cli, db, mode), script)
        if p is None:
            o = ("timeout", None)
        else:
            e = cli_error(p.stderr, line)
            if e is not None:
                o = ("err", e)
            else:
                try:
                    o = ("ok", parse_quote(p.stdout))
                except (Unparsed, ValueError) as x:
                    o = ("err", f"pretriage: unreadable CLI output: {x}")
        if o[0] != "ok" or len(o[1]) <= CACHE_ROWS:
            self.cache[k] = o
        if o[0] == "ok":
            self.rows_seen = max(self.rows_seen, len(o[1]))
        return o

    def run_all(self, cli, db, db_digest, stmts, mode="file", opts=None):
        """Every statement's rows concatenated, or the first error
        (sqlancer.rs concat())."""
        rows = []
        for s in stmts:
            o = self.run(cli, db, db_digest, s, mode, opts)
            if o[0] != "ok":
                return o
            rows.extend(o[1])
        return ("ok", rows)

    def jsonb(self, cli, outcomes):
        """The outcomes with every BLOB that is valid JSONB (json_valid(x, 8))
        as its JSON text (json(x)), by the same CLI (sqlancer.rs run())."""
        blobs = sorted({c[1] for o in outcomes if o and o[0] == "ok" for r in o[1] for c in r if c[0] == "b"})
        if not blobs:
            return outcomes
        k = (cli.path, "jsonb", tuple(blobs))
        if k not in self.cache:
            script = ".mode quote\n" + "".join(f"SELECT {n}, json(x'{b.hex()}') WHERE json_valid(x'{b.hex()}', 8);\n" for n, b in enumerate(blobs))
            p = self._exec(cli, ":memory:", script.encode())
            text = {}
            if p is not None:
                try:
                    for r in parse_quote(p.stdout):
                        if len(r) == 2 and r[0][0] == "i" and r[1][0] == "t":
                            text[blobs[r[0][1]]] = r[1][1]
                except (Unparsed, ValueError, IndexError):
                    pass
            self.cache[k] = text
        text = self.cache[k]
        conv = lambda c: ("j", text[c[1]], c[1]) if c[0] == "b" and c[1] in text else c
        return [(o[0], [[conv(c) for c in r] for r in o[1]]) if o and o[0] == "ok" else o for o in outcomes]


# -------------------------------------------------------------- registry


def read_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def read_text(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read().strip()


def load_registry(path):
    return read_json(path)


def open_bugs(registry):
    return [b for b in registry["bugs"] if b.get("status") == "open"]


def load_clis(clis_dir, registry):
    """The trunk CLI and the patched CLIs (bug name -> Cli) of the CLIs dir
    (RUN_ROOT/clis/current), the check-in, and the patched CLIs' statuses."""
    real = os.path.realpath(clis_dir)
    checkin = os.path.basename(real)
    trunk_path = os.path.join(real, "trunk", "sqlite3")
    if not os.access(trunk_path, os.X_OK):
        raise SystemExit(f"pretriage: no trunk CLI at {trunk_path} (run build-clis.sh)")
    trunk = Cli("trunk", trunk_path, checkin)
    patched, status = {}, {}
    for b in open_bugs(registry):
        if not b.get("fix"):
            status[b["name"]] = "no-fix"
            continue
        d = os.path.join(real, b["name"])
        st = read_text(os.path.join(d, "status")) if os.path.exists(os.path.join(d, "status")) else "missing"
        status[b["name"]] = st
        if st == "ok" and os.access(os.path.join(d, "sqlite3"), os.X_OK):
            patched[b["name"]] = Cli(b["name"], os.path.join(d, "sqlite3"), checkin, reindex=bool(b.get("reindex")))
    version = None
    vf = os.path.join(real, "version")
    if os.path.exists(vf):
        version = read_text(vf)
    return trunk, patched, status, version


def error_matches(bug, error, sql):
    """Whether a reproduced failure is a bug without a fix (its `match`):
    the trunk error contains `error` (when it is null, the failure is a
    wrong result, and any reproduced failure goes on to the patterns), the
    SQL matches every pattern of `sql_all` and, when given, one of
    `sql_any`."""
    m = bug.get("match") or {}
    if m.get("error") is not None and m["error"] not in (error or ""):
        return False
    if m.get("error") is None and not (m.get("sql_all") or m.get("sql_any")):
        return False
    return all(re.search(p, sql) for p in m.get("sql_all", [])) and (not m.get("sql_any") or any(re.search(p, sql) for p in m["sql_any"]))


def text_patterns(sql):
    """The LIKE and GLOB patterns of the SQL that are text literals with a
    non-ASCII character before their first wildcard (sqlancer.rs
    invalid_text_pattern(), its first half), and the tables it reads."""
    toks = scan(sql)
    pats = []
    for n, tok in enumerate(toks[:-1]):
        if _is(tok, "LIKE", "GLOB") and toks[n + 1][0] == "str" and toks[n + 1][1].startswith("'"):
            p = toks[n + 1][1][1:-1].replace("''", "'")
            wild = "*?[" if tok[1].upper() == "GLOB" else "%_"
            prefix = p[: next((k for k, ch in enumerate(p) if ch in wild), len(p))]
            if any(ord(ch) > 127 for ch in prefix):
                pats.append(p)
    tables = sorted({tok[1] for tok in toks if tok[0] == "word" and re.fullmatch(r"t\d+", tok[1])})
    return pats, tables


def invalid_text(b, encoding):
    """Whether a blob is not valid text in the database's encoding."""
    try:
        b.decode({"UTF-8": "utf-8", "UTF-16le": "utf-16-le", "UTF-16be": "utf-16-be"}.get(encoding, "utf-8"))
        return False
    except UnicodeDecodeError:
        return True


# ---------------------------------------------------------------- triage


# Connection settings among a case's statements that persist to its queries
# and change their plans, but are not stored in the database file.
CONNECTION_PRAGMAS = re.compile(r"PRAGMA (automatic_index|reverse_unordered_selects) = ([01])\b")


def connection_pragmas(input_text):
    """The script that restores a case's connection settings (the last value
    of each of CONNECTION_PRAGMAS in its input.txt), b"" if it sets none."""
    last = {}
    for m in CONNECTION_PRAGMAS.finditer(input_text):
        last[m.group(1)] = m.group(2)
    return "".join(f"PRAGMA {k} = {v};\n" for k, v in sorted(last.items())).encode()


# ---------------------------------------------------- the case's statements
#
# after.db is the main database only, as committed: the runner serializes
# the case's connection after its statements (sqlite3_serialize of an
# in-memory main returns its committed pages), so it holds neither a TEMP
# schema, nor an ATTACHed database (`ATTACH ':memory:' AS aux`), nor what
# the statements after a transaction they leave open wrote (`BEGIN;
# SAVEPOINT s0; ...` with no COMMIT: the queries ran inside it). A finding
# of such a case replayed on after.db alone fails with "no such table:
# aux.t6" or "no such column: c1" (run 20261021-0443's plans w5-3089,
# w6-4286 and w13-61234, all sqlite-wr-rowvalue-added-column). Such a case
# is replayed with its statements: on the case's own starting database,
# before.db, with case.sql (the runner writes both since that run:
# faithful), or, for an example from before, on after.db with the
# statements in input.txt (`reconstructed`, approximate: the open
# transaction's statements on after.db, which is the database as it was
# when the transaction began; for a TEMP or ATTACHed database, every
# statement with main read-only, so that only the other databases change,
# from main's final contents).


def schema_mismatch(replayed, original):
    """Whether a replay's error says the schema is not the one the run had
    (no such table or column) while the run's own outcome did not: a replay
    on the wrong database, not the failure."""
    m = re.match(r"no such (?:table|column): ", replayed or "")
    return bool(m) and not (original or "").startswith(m.group())


def case_script(input_text):
    """The case's statements from an input.txt (the runner's Debug of the
    case: `  -- N statements`, each statement as `  <sql>;`, then `}` and
    the queries), as an SQL script; None when there is none."""
    m = re.search(r"^  -- \d+ statements\n", input_text, re.M)
    if not m:
        return None
    rest = input_text[m.end() :]
    if rest.startswith("}"):
        return ""
    # The case's closing brace: the first `}` at a line's start outside a
    # literal.
    for kind, text, _, s, _ in scan(rest):
        if text == "}" and rest[s - 1] == "\n":
            return rest[:s]
    return None


def split_statements(script):
    """The script's statements as (start, end, words): their places in it
    and their first few words, upper-cased (a CREATE TRIGGER's body, whose
    statements end in `;` too, is part of it, through its END)."""
    out = []
    start, words, in_trigger = None, [], False
    for kind, text, depth, s, e in scan(script):
        if start is None:
            if text == ";":
                continue
            start, words, in_trigger = s, [], False
        if kind == "word" and len(words) < 6:
            words.append(text.upper())
            if words[0] == "CREATE" and text.upper() == "TRIGGER":
                in_trigger = True
        if text == ";" and depth == 0:
            if in_trigger:
                # A trigger ends at an END that is a body statement's own
                # (`...; END;`), not a CASE's.
                body = script[start:s].rstrip()
                if not re.search(r"(?is);\s*END$", body):
                    continue
            out.append((start, s, tuple(words)))
            start = None
    if start is not None:
        out.append((start, len(script), tuple(words)))
    return out


def _name(word):
    return word.strip('"`[]').lower()


def case_needs(script):
    """What after.db lacks of a case's connection: {"attach": an ATTACHed
    database still attached, "temp": a TEMP object made, "tail": the
    script's offset where the transaction it leaves open began, or None}."""
    attach = temp = False
    tail, opened_by, savepoints = None, None, []
    for start, end, w in split_statements(script):
        sql = script[start:end]
        first = w[0] if w else ""
        if first == "ATTACH":
            attach = True
        elif first == "DETACH":
            attach = False
        if first == "CREATE" and (len(w) > 1 and w[1] in ("TEMP", "TEMPORARY") or re.match(r"(?is)CREATE\s+(?:UNIQUE\s+|VIRTUAL\s+)?\w+\s+(?:IF\s+NOT\s+EXISTS\s+)?[\"`\[]?temp[\"`\]]?\s*\.", sql)):
            temp = True
        if first == "BEGIN":
            if opened_by is None:
                opened_by, tail, savepoints = "begin", start, []
        elif first in ("COMMIT", "END") or (first == "ROLLBACK" and "TO" not in w[1:3]):
            opened_by, tail, savepoints = None, None, []
        elif first == "SAVEPOINT" and len(w) > 1:
            if opened_by is None:
                opened_by, tail = "savepoint", start
            savepoints.append(_name(sql.split()[1]))
        elif first == "RELEASE":
            toks = sql.split()
            name = _name(toks[2] if len(toks) > 2 and toks[1].upper() == "SAVEPOINT" else toks[1]) if len(toks) > 1 else ""
            if name in savepoints:
                del savepoints[len(savepoints) - 1 - savepoints[::-1].index(name) :]
                if not savepoints and opened_by == "savepoint":
                    opened_by, tail = None, None
    return {"attach": attach, "temp": temp, "tail": tail}


class Triage:
    def __init__(self, trunk, patched, registry, clis_root, replayer):
        self.trunk, self.patched, self.registry = trunk, patched, registry
        self.clis_root, self.r = clis_root, replayer
        self.match_bugs = [b for b in open_bugs(registry) if not b.get("fix") and b.get("match")]
        # The bugs whose fixes the run's SQLite has (run.json's
        # config.fixed_bugs, SQLITE_TARGET=fixed), set per findings dir.
        self.run_fixes = []
        # The current example's replay mode (MODES), from its storage.
        self.mode = "file"

    def _run(self, cli, db, digest, sql, mode=None):
        return self.r.run(cli, db, digest, sql, mode or self.mode)

    def _run_all(self, cli, db, digest, stmts, mode=None, opts=None):
        if opts is None:
            return self.r.run_all(cli, db, digest, stmts, mode or self.mode)
        return self.r.run_all(cli, db, digest, stmts, mode or self.mode, opts)

    def target_cli(self, target):
        if not target:
            return None
        p = os.path.join(self.clis_root, target, "trunk", "sqlite3")
        return Cli("target", p, target) if os.access(p, os.X_OK) else None

    def fixed_cli(self, target):
        """The run's own SQLite on a fixed run: build-fixed.sh's CLI of the
        target check-in with the known bugs' fixes (RUN_ROOT/clis/<target>/
        fixed), or None."""
        if not target:
            return None
        p = os.path.join(self.clis_root, target, "fixed", "sqlite3")
        return Cli("fixed", p, target) if os.access(p, os.X_OK) else None

    def _fixed_ev(self, cli, ev):
        """Evidence of a replay on the fixed CLI, with the fixes it has
        (its fixes.json) when they are not the run's."""
        try:
            has = read_json(os.path.join(os.path.dirname(cli.path), "fixes.json")).get("applied", [])
        except (OSError, ValueError):
            has = None
        if has is not None and sorted(has) != sorted(self.run_fixes):
            ev = dict(ev, cli_fixes=has, note="the fixed CLI has other fixes than the run's SQLite (rebuilt since)")
        return ev

    def example(self, edir):
        """The verdict of one example dir: a dict for triage.json."""
        with open(os.path.join(edir, "message.txt"), encoding="utf-8", errors="replace") as f:
            text = f.read()
        ex = {}
        try:
            ex = read_json(os.path.join(edir, "example.json"))
        except (OSError, ValueError):
            pass
        target = ex.get("target")
        storage = ex.get("storage") or "memory"
        self.mode = "wal" if storage == "wal" else "file"
        db = os.path.join(edir, "after.db")
        if not os.path.exists(db):
            db = None
        digest = "-"
        if db:
            with open(db, "rb") as f:
                digest = hashlib.sha1(f.read()).hexdigest()
        base = {"trunk": self.trunk.checkin, "target": target, "first_line": text.split("\n", 1)[0][:200], "replay": self.mode}
        self.r.new_example()
        try:
            input_text = read_text(os.path.join(edir, "input.txt"))
        except OSError:
            input_text = ""
        prelude = connection_pragmas(input_text)
        self.r.prelude = prelude
        if prelude:
            base["pragmas"] = prelude.decode().strip().split("\n")
        case = self.case_replay(edir, input_text, ex)
        if case:
            base["case_replay"] = case
        if re.match(r"statement \d+: ", text) and nondeterministic(text):
            return dict(base, verdict="excluded", bugs=[], reason="a case's statement refused for a date function reading the current time (nondeterministic), which the current oracles skip: " + text.rstrip()[-160:])
        try:
            msg = parse_message(text)
        except Unparsed as e:
            return dict(base, verdict="manual", bugs=[], reason=f"unparsed: {e}")
        base["kind"] = msg["kind"]
        if msg.get("variant"):
            base["variant"] = msg["variant"]
        issue = self.known_issue(msg, db, digest)
        if issue:
            return dict(base, verdict="excluded", bugs=[], reason=issue)
        try:
            if msg["kind"] == "fails":
                v = self._fails(msg, db, digest, target)
            elif msg["kind"] == "differ":
                v = self._differ(msg, db, digest, target)
            else:
                v = self._order(msg, db, digest, target)
        except Unparsed as e:
            v = {"verdict": "manual", "bugs": [], "reason": f"unparsed: {e}"}
        return dict(base, **v)

    def case_replay(self, edir, input_text, ex):
        """When after.db is not the case's connection (a TEMP or ATTACHed
        database, or a transaction the case leaves open: case_needs), the
        replay runs the case's statements before every statement: on
        before.db with case.sql (faithful), else on after.db with
        input.txt's statements (reconstructed). What it does, for
        triage.json, or None."""
        before, case_sql = os.path.join(edir, "before.db"), os.path.join(edir, "case.sql")
        if os.path.exists(case_sql):
            with open(case_sql, "rb") as f:
                script = f.read().decode("utf-8", "surrogateescape")
        else:
            script = case_script(input_text)
        if not script:
            return None
        needs = case_needs(script)
        what = [k for k, on in (("TEMP", needs["temp"]), ("ATTACH", needs["attach"]), ("open transaction", needs["tail"] is not None)) if on]
        if not what:
            return None
        raw = script.encode("utf-8", "surrogateescape")
        if os.path.exists(before) and os.path.exists(case_sql):
            self.r.set_case(raw, before=before, wal=ex.get("wal_settings"))
            return {"how": "faithful", "needs": what, "note": "every statement replayed after case.sql on before.db, as the runner opened it"}
        if needs["attach"] or needs["temp"]:
            self.r.set_case(raw, readonly=True)
            note = "every statement replayed after input.txt's case statements on after.db with main read-only: the TEMP and ATTACHed databases filled from main's final contents"
            if needs["tail"] is not None:
                note += "; the open transaction's writes to main are lost"
        else:
            self.r.set_case(script[needs["tail"] :].encode("utf-8", "surrogateescape"))
            note = "every statement replayed after input.txt's statements from the transaction the case leaves open on, on after.db (the database as committed before it)"
        return {"how": "reconstructed", "needs": what, "note": note + " (an example from before the runner wrote before.db and case.sql)"}

    def known_issue(self, msg, db, digest):
        """sqlancer.rs known_issue(): a query the current oracles skip for
        known SQLite behaviour (invalid_text_pattern: a LIKE or GLOB whose
        text pattern has a non-ASCII character before its first wildcard,
        reading a table that holds a blob that is not valid text in the
        database's encoding; bugs/sqlite/sqlite-glob-index-blob, not a bug). The
        reason, or None."""
        sql = "\n".join(msg.get(k, "") for k in ("osql", "vsql", "sql"))
        pats, tables = text_patterns(sql)
        if not pats or not db:
            return None
        enc = self._run(self.trunk, db, digest, "PRAGMA encoding")
        encoding = enc[1][0][0][1].decode() if enc[0] == "ok" and enc[1] and enc[1][0][0][0] == "t" else "UTF-8"
        for tab in tables:
            o = self._run(self.trunk, db, digest, f"SELECT * FROM {tab}")
            if o[0] == "ok" and any(c[0] == "b" and invalid_text(c[1], encoding) for row in o[1] for c in row):
                return (f"a query the current oracles skip as known SQLite behaviour (invalid_text_pattern: pattern {pats[0][:40]!r} "
                        f"and a blob in {tab} that is not {encoding} text; bugs/sqlite/sqlite-glob-index-blob)")
        return None

    # Each kind: reproduces(cli) -> (True/False/None, evidence), and
    # explained(cli, trunk evidence) -> (bool, evidence).

    def _decide(self, reproduces, explained, sql, target, trunk_error=None):
        ok, ev = reproduces(self.trunk)
        if ok is None:
            return {"verdict": "manual", "bugs": [], "reason": "trunk replay: " + ev.get("why", "undecided"), "evidence": {"trunk": ev}}
        if ok:
            # The patched CLIs side by side (each on its own database copy).
            # One at a time when the outputs are large (their rows in memory).
            names = sorted(self.patched)
            workers = 1 if getattr(self.r, "rows_seen", 0) > BIG_ROWS else max(1, min(8, len(names)))
            with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
                results = list(pool.map(lambda n: explained(self.patched[n], ev), names))
            pev = {n: r[1] for n, r in zip(names, results)}
            bugs = [n for n, r in zip(names, results) if r[0]]
            # A `reindex` bug is credited only if trunk still reproduces the
            # failure on a copy its own REINDEX rebuilt: else the rebuild,
            # not the fix, made it go away.
            if any(self.patched[n].reindex for n in bugs):
                control = Cli("trunk-reindex", self.trunk.path, self.trunk.checkin, reindex=True)
                cok, cev = reproduces(control)
                if cok is not True:
                    for n in [n for n in bugs if self.patched[n].reindex]:
                        bugs.remove(n)
                        pev[n] = dict(pev[n], why="trunk does not reproduce it after REINDEX either", control=cev)
            matched = [b["name"] for b in self.match_bugs if error_matches(b, trunk_error(ev) if trunk_error else None, sql)]
            explaining = {n: pev[n] for n in bugs}
            if bugs or matched:
                reason = "trunk reproduces it; " + ", ".join([f"{n}'s fix makes it go away" for n in bugs] + [f"{n} matches its error" for n in matched])
                v = {"verdict": "known", "bugs": bugs + matched, "reason": reason, "evidence": {"trunk": ev, "patched": explaining}}
                # On a fixed run, a known bug whose fix the run's SQLite has
                # should not have been found: its fix is incomplete (the
                # fixed CLI reproduces it too), or the replay is not the run.
                had = [n for n in bugs + matched if n in self.run_fixes]
                if had:
                    fcli = self.fixed_cli(target)
                    fok, fev = reproduces(fcli) if fcli else (None, None)
                    if fcli:
                        v["evidence"]["fixed"] = self._fixed_ev(fcli, fev)
                    v["fix_incomplete"] = had
                    v["note"] = (f"the run's SQLite has the fix of {', '.join(had)} and found it anyway: "
                                 + ("the fixed CLI reproduces it too, so the fix is incomplete" if fok else
                                    "the fixed CLI does not reproduce it (the run's own execution differs from the replay)" if fok is False else
                                    "no fixed CLI to tell whether the fix is incomplete"))
                    v["reason"] += "; " + v["note"]
                return v
            return {"verdict": "new", "bugs": [], "reason": "trunk reproduces it and no known bug's fix makes it go away",
                    "evidence": {"trunk": ev, "patched": {n: e for n, e in list(pev.items())[:3]}}}
        if not target:
            return {"verdict": "manual", "bugs": [], "reason": f"trunk {self.trunk.checkin} does not reproduce it, and the example records no target check-in", "evidence": {"trunk": ev}}
        if self.run_fixes:
            v = self._decide_fixed(reproduces, ev, target)
            if v:
                return v
        if self.trunk.checkin.startswith(target) or target.startswith(self.trunk.checkin):
            return {"verdict": "not-reproducible", "bugs": [], "reason": f"the CLI of the run's target {target} does not reproduce it", "evidence": {"trunk": ev}}
        tcli = self.target_cli(target)
        if tcli is None:
            return {"verdict": "fixed-upstream", "bugs": [],
                    "reason": f"trunk {self.trunk.checkin} does not reproduce it; the message shows it failed on the target {target} (no CLI of the target to confirm: it may also be not-reproducible)",
                    "evidence": {"trunk": ev, "target_cli": None}}
        tok, tev = reproduces(tcli)
        if tok:
            return {"verdict": "fixed-upstream", "bugs": [], "reason": f"trunk {self.trunk.checkin} does not reproduce it, the run's target {target} does",
                    "evidence": {"trunk": ev, "target": tev}}
        if tok is None:
            return {"verdict": "manual", "bugs": [], "reason": "target replay: " + tev.get("why", "undecided"), "evidence": {"trunk": ev, "target": tev}}
        return {"verdict": "not-reproducible", "bugs": [], "reason": f"neither trunk {self.trunk.checkin} nor the run's target {target} reproduces it",
                "evidence": {"trunk": ev, "target": tev}}

    def _decide_fixed(self, reproduces, ev, target):
        """A fixed run's finding that trunk does not reproduce, replayed on
        the run's own SQLite (the target with the known bugs' fixes): when
        that reproduces it and the plain target does not (it is trunk, or
        its CLI does not reproduce it), `fixes-only`: a bug of one of our
        fixes, or one they expose together; when the plain target
        reproduces it too, `fixed-upstream`; when the fixed CLI does not,
        `not-reproducible`. None (the plain-trunk rules) without a fixed CLI."""
        fcli = self.fixed_cli(target)
        fixes = ", ".join(self.run_fixes)
        if fcli is None:
            return None
        fok, fev = reproduces(fcli)
        fev = self._fixed_ev(fcli, fev)
        if fok is None:
            return {"verdict": "manual", "bugs": [], "reason": "fixed CLI replay: " + fev.get("why", "undecided"), "evidence": {"trunk": ev, "fixed": fev}}
        if not fok:
            return {"verdict": "not-reproducible", "bugs": [], "reason": f"neither trunk {self.trunk.checkin} nor the run's own SQLite ({target} with the fixes of {fixes}) reproduces it",
                    "evidence": {"trunk": ev, "fixed": fev}}
        same = self.trunk.checkin.startswith(target) or target.startswith(self.trunk.checkin)
        tcli = None if same else self.target_cli(target)
        if tcli is not None:
            tok, tev = reproduces(tcli)
            if tok:
                return {"verdict": "fixed-upstream", "bugs": [], "reason": f"trunk {self.trunk.checkin} does not reproduce it, the run's target {target} does, with the fixes and without",
                        "evidence": {"trunk": ev, "fixed": fev, "target": tev}}
            if tok is None:
                return {"verdict": "manual", "bugs": [], "reason": "target replay: " + tev.get("why", "undecided"), "evidence": {"trunk": ev, "fixed": fev, "target": tev}}
        plain = f"trunk {self.trunk.checkin}" if same else f"trunk {self.trunk.checkin} and the plain target {target}" if tcli else f"trunk {self.trunk.checkin} (no CLI of the plain target {target})"
        return {"verdict": "fixes-only", "bugs": [], "fixes": list(self.run_fixes),
                "reason": f"the run's own SQLite ({target} with the fixes of {fixes}) reproduces it, {plain} does not: a bug in one of the fixes, or one they expose",
                "evidence": {"trunk": ev, "fixed": fev, **({"target": tev} if tcli else {})}}

    def _fails(self, msg, db, digest, target):
        e = msg["error"]
        c = skip_class(e)
        if c:
            return {"verdict": "excluded", "bugs": [], "reason": f"{c} error, which the current oracles skip: {e[:200]}"}
        sql = msg["sql"]

        def reproduces(cli):
            o = self._run(cli, db, digest, sql)
            ev = {"cli": cli.checkin, "outcome": fmt_outcome(o)}
            if o[0] == "timeout":
                return None, dict(ev, why="timeout")
            if o[0] == "err" and schema_mismatch(o[1], e):
                return None, dict(ev, why=f"the replay's database is not the case's: {o[1][:120]} (the run's error: {e[:120]})")
            if o[0] == "err" and not skip_class(o[1]):
                return True, dict(ev, error=o[1])
            return False, ev

        def explained(cli, _):
            o = self._run(cli, db, digest, sql)
            return o[0] == "ok", {"outcome": fmt_outcome(o, 200)}

        return self._decide(reproduces, explained, sql, target, trunk_error=lambda ev: ev.get("error"))

    def _sides(self, msg, cli, db, digest):
        """The two outcomes of a comparison on cli, as the oracle forms them
        (the storage oracle's original in memory, its variant in a WAL
        file; plans-noopt's variant, the same SQL, with the optimizations of
        the message's mask off)."""
        variant = msg["variant"]
        omode, vmode = ("memory", "wal") if variant == "storage" else (None, None)
        if "slice" in msg:
            full = self._run(cli, db, digest, msg["osql"])
            k, n = msg["slice"]
            k = max(k, 0)
            o = ("ok", full[1][k : (None if n < 0 else k + n)]) if full[0] == "ok" else full
        else:
            o = self._run_all(cli, db, digest, [msg["osql"]], omode)
        # plans-noopt: the same SQL with the message's optimizations off.
        v = self._run_all(cli, db, digest, msg["vstmts"], vmode, msg.get("opts"))
        if variant in ("norec", "norec-join") and v[0] == "ok" and v[1] == [[NULL]]:
            v = ("ok", [[("i", 0)]])
        if variant == "order-sorted" and v[0] == "ok" and o[0] == "ok" and o[1]:
            w = len(o[1][0])
            v = ("ok", [r[:w] for r in v[1]])
        o, v = self.r.jsonb(cli, [o, v])
        return o, v

    def folded(self, sql, db, digest):
        """sqlancer.rs folds(), as the SQL and the database tell: the query
        puts an expression under NOCASE or RTRIM, or a table it reads
        declares a column so. The oracle then compared text folded and
        computed set operations under each column's sequence, which the
        recomputation here does not model."""
        coll = re.compile(r"(?i)\bCOLLATE\s+(?:NOCASE|RTRIM)\b")
        if coll.search(sql):
            return True
        if not db:
            return False
        o = self._run(self.trunk, db, digest, "SELECT name, sql FROM sqlite_schema WHERE type = 'table'")
        tables = {tok[1] for tok in scan(sql) if tok[0] == "word" and re.fullmatch(r"t\d+", tok[1])}
        return o[0] == "ok" and any(r[0] == ("t", t.encode()) and r[1][0] == "t" and coll.search(r[1][1].decode("utf-8", "replace"))
                                    for r in o[1] if len(r) == 2 for t in tables)

    def reference(self, msg, db, digest):
        """For setops-def and window-def, whose reference side the runner
        computed in Rust from other queries' rows: that side recomputed on a
        CLI from the same queries (Reference), or None where pre-triage does
        not model it (setops-def compared folded, a window it cannot read);
        then each patched CLI still counts when it changes those queries'
        rows (Reference.changed)."""
        tri = self
        if msg["variant"] == "setops-def":
            try:
                parts = compound_parts(msg["vsql"])
            except Unparsed:
                return None
            equiv = unordered_mode(msg["vsql"]) != "multiset"
            stmts = parts["cores"]
            compute = None if self.folded(msg["vsql"], db, digest) else (lambda outs: setops_reference(parts, outs, equiv))
        elif msg["variant"] == "window-def":
            bsql = msg["vsql"][: -len(_WINDOW_WHAT)]
            stmts = [bsql]
            try:
                items = _items(msg["osql"])
                windows = [window_item(it) for it in items]
            except Unparsed:
                windows = None
            compute = None if not windows or not any(windows) else (lambda outs: windows_reference(windows, len(items), outs[0]))
        else:
            return None

        class Reference:
            def inputs(self, cli):
                return [tri._run(cli, db, digest, sq) for sq in stmts]

            def __call__(self, cli):
                if compute is None:
                    return None
                return compute(self.inputs(cli))

            def changed(self, cli, trunk):
                """How the CLI's reference side differs from trunk's: its
                recomputed rows (compared as the oracle compares), or, where
                it is not recomputed, a query's rows (as a multiset)."""
                if compute is not None:
                    p, t = self(cli), self(trunk)
                    if p is None or t is None or "timeout" in (p[0], t[0]):
                        return None
                    p, t = tri.r.jsonb(cli, [p, t])
                    same = (p[0] == t[0] == "ok" and compare_rows("multiset", p[1], t[1])) or (p[0] == t[0] == "err")
                    return None if same else "recomputed"
                for k, (p, t) in enumerate(zip(self.inputs(cli), self.inputs(trunk))):
                    if "timeout" in (p[0], t[0]):
                        continue
                    if p[0] != t[0] or (p[0] == "ok" and not compare_rows("multiset", p[1], t[1])):
                        return f"query {k}"
                return None

        return Reference()

    def _differ(self, msg, db, digest, target):
        variant = msg["variant"]
        mo, mv = parse_outcome(msg["orig"]), parse_outcome(msg["var"])
        for side in (mo, mv):
            if side[0] == "err" and skip_class(side[1]) and (mo[0] == "ok" or mv[0] == "ok"):
                return {"verdict": "excluded", "bugs": [], "reason": f"one side fails with a {skip_class(side[1])} error, which the current oracles skip: {side[1][:200]}"}
        mode = infer_mode(variant, msg["osql"], msg["vsql"])
        exp_side = msg.get("expected_side")
        if mo[0] == "ok" and mv[0] == "ok":
            # The message's own results, compared as the current oracles do.
            if not compare_rows(mode, mo[1], mv[1]):
                no, nv = self.r.jsonb(self.trunk, [mo, mv])
                if compare_rows(mode, no[1], nv[1]):
                    sample = {"original_only": fmt_outcome(("ok", rows_only(mo[1], mv[1])[:1]), 300),
                              "variant_only": fmt_outcome(("ok", rows_only(mv[1], mo[1])[:1]), 300)}
                    return {"verdict": "jsonb-equivalent", "bugs": [], "mode": mode,
                            "reason": "the message's two results differ only in the bytes of JSONB values: they agree once JSONB compares as json(x), as the current oracles compare",
                            "evidence": {"message": {"original": fmt_outcome(mo), "variant": fmt_outcome(mv)}, "rows_apart": sample}}
        if exp_side:
            expected = mo if exp_side == "orig" else mv
            actual_sql = [msg["vsql"]] if exp_side == "orig" else [msg["osql"]]
            ref = self.reference(msg, db, digest)

            def sides(cli):
                """The actual side, the reference side recomputed (or the
                message's expected rows where it cannot be), JSONB
                normalized, and the reference's own outcome (None)."""
                a = self._run_all(cli, db, digest, actual_sql)
                r = ref(cli) if ref else None
                e = r if r is not None else expected
                a, e = self.r.jsonb(cli, [a, e])
                return a, e, r

            def reproduces(cli):
                a, e, r = sides(cli)
                ok, why = agree(mode, e, a)
                ev = {"cli": cli.checkin, "mode": mode, "actual": fmt_outcome(a), "expected": fmt_outcome(expected), "why": why}
                if r is not None:
                    ev["reference"] = fmt_outcome(e)
                return (None if ok is None else not ok), ev

            def explained(cli, tev):
                a, e, r = sides(cli)
                ok, why = agree(mode, e, a)
                if ok:
                    return True, {"actual": fmt_outcome(a, 200), "reference": fmt_outcome(e, 200), "rule": "recomputed" if r is not None else "expected"}
                if r is not None and agree(mode, *self.r.jsonb(cli, [expected, a]))[0]:
                    return True, {"actual": fmt_outcome(a, 200), "rule": "expected"}
                ta, te, tr = sides(self.trunk)
                if a[0] != "timeout" and ta[0] != "timeout" and fmt_outcome(a, 10 ** 9) != fmt_outcome(ta, 10 ** 9):
                    return True, {"actual": fmt_outcome(a, 200), "rule": "changed"}
                # A fix that changes the reference side counts as one that
                # changes the actual side does.
                changed = ref.changed(cli, self.trunk) if ref else False
                if changed:
                    return True, {"actual": fmt_outcome(a, 200), "reference": fmt_outcome(e, 200), "rule": "changed-reference", "changed": changed}
                return False, {"actual": fmt_outcome(a, 200), "reference": fmt_outcome(e, 200) if r is not None else None}

            return dict(self._decide(reproduces, explained, msg["vsql"] if exp_side == "orig" else msg["osql"], target), mode=mode)

        def reproduces(cli):
            o, v = self._sides(msg, cli, db, digest)
            ok, why = agree(mode, o, v)
            ev = {"cli": cli.checkin, "mode": mode, "original": fmt_outcome(o), "variant": fmt_outcome(v), "why": why}
            if "opts" in msg:
                ev["optimizations_off"] = f"{msg['opts']:#010x} ({msg['opts_names']})"
            for side, run in ((o, mo), (v, mv)):
                if side[0] == "err" and schema_mismatch(side[1], run[1] if run[0] == "err" else ""):
                    return None, dict(ev, why=f"the replay's database is not the case's: {side[1][:120]}")
            if ok is None:
                return None, ev
            if not ok:
                ev["error"] = o[1] if o[0] == "err" else (v[1] if v[0] == "err" else None)
            return (not ok), ev

        def explained(cli, _):
            o, v = self._sides(msg, cli, db, digest)
            ok, why = agree(mode, o, v)
            yes = bool(ok) and not (o[0] == "err" and v[0] == "err")
            return yes, {"original": fmt_outcome(o, 200), "variant": fmt_outcome(v, 200), "why": why}

        sql = msg["osql"] + "\n" + msg["vsql"]
        return dict(self._decide(reproduces, explained, sql, target, trunk_error=lambda ev: ev.get("error")), mode=mode)

    def _order(self, msg, db, digest, target):
        sql = msg["sql"]
        terms = Shape(sql).order
        if not terms:
            return {"verdict": "manual", "bugs": [], "reason": "no top-level ORDER BY found in the query"}

        def reproduces(cli):
            o = self._run(cli, db, digest, sql)
            ev = {"cli": cli.checkin, "outcome": fmt_outcome(o)}
            if o[0] != "ok":
                return None, dict(ev, why="the query fails" if o[0] == "err" else "timeout")
            n = out_of_order(o[1], terms)
            if n is None:
                return False, dict(ev, why="in order")
            return True, dict(ev, why=f"rows {n} and {n + 1} out of order")

        def explained(cli, _):
            o = self._run(cli, db, digest, sql)
            return o[0] == "ok" and out_of_order(o[1], terms) is None, {"outcome": fmt_outcome(o, 200)}

        return dict(self._decide(reproduces, explained, sql, target), mode="ordered")


# ------------------------------------------------------------- outputs


# Verdicts that alert (explained:false): no known bug explains them.
UNEXPLAINED = ("new", "fixes-only", "manual")


def run_fixes(out_dir):
    """The bugs whose fixes the run's SQLite has (run.json's
    config.fixed_bugs, start-run.sh's --fixed-bugs), [] for plain trunk."""
    try:
        return list(read_json(os.path.join(out_dir, "run.json")).get("config", {}).get("fixed_bugs") or [])
    except (OSError, ValueError, AttributeError):
        return []


def label(t):
    return "known:" + "+".join(t["bugs"]) if t["verdict"] == "known" else t["verdict"]


def append_line(path, obj):
    line = json.dumps(obj, ensure_ascii=False, separators=(",", ":")) + "\n"
    with open(path, "a", encoding="utf-8") as f:
        f.write(line)


def write_atomic(path, text):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text)
    os.replace(tmp, path)


def truncate(s, n):
    return s if s is None or len(s) <= n else s[: n - 3] + "..."


class StatsD:
    def __init__(self, addr):
        self.addr = None
        if addr:
            host, _, port = addr.rpartition(":")
            self.addr = (host or "127.0.0.1", int(port))
            self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

    def count(self, name, n, tags):
        if self.addr and n:
            try:
                self.sock.sendto(f"{name}:{n}|c|#{tags}".encode(), self.addr)
            except OSError:
                pass


def identity(out_dir, example_json):
    """The run's identity (records.rs Identity) and its DogStatsD tags."""
    ident = {}
    try:
        j = read_json(os.path.join(out_dir, "run.json"))
        ident = {"run": j["run_id"], "campaign": j["campaign"], "work_tag": j["work_tag"], "version": j["version_id"], "target": j["target_id"]}
    except (OSError, ValueError, KeyError):
        if example_json:
            ident = {k: example_json.get(k) for k in ("run", "campaign", "work_tag", "version", "target")}
    return ident


def run_tags(ident, extra):
    tags = "run:{run},campaign:{campaign},work_tag:{work_tag},version:{version},target:sqlite-{target}".format(**{k: ident.get(k) or "unknown" for k in ("run", "campaign", "work_tag", "version", "target")})
    return tags + ("," + extra if extra else "")


def find_targets(paths, root, all_runs):
    """(findings dir, output dir) pairs: a run dir's findings/ (output: the
    run dir), or a findings dir itself (output: its run dir when it is one's
    findings/, else itself)."""
    out = []
    if all_runs:
        for name in sorted(os.listdir(root)):
            d = os.path.join(root, name)
            if name in ("current", "clis") or os.path.islink(d) or not os.path.isdir(os.path.join(d, "findings")):
                continue
            out.append((os.path.join(d, "findings"), d))
        return out
    for p in paths or [os.path.join(root, "current")]:
        p = os.path.realpath(p)
        if os.path.isdir(os.path.join(p, "findings")):
            out.append((os.path.join(p, "findings"), p))
        elif os.path.isdir(p):
            parent = os.path.dirname(p)
            out.append((p, parent if os.path.basename(p) == "findings" and os.path.exists(os.path.join(parent, "run.json")) else p))
        else:
            print(f"pretriage: no run or findings dir at {p}", file=sys.stderr)
    return out


def examples(findings):
    for sig in sorted(os.listdir(findings)):
        sdir = os.path.join(findings, sig)
        if sig in LOST or not os.path.isdir(sdir):
            continue
        for ex in sorted(os.listdir(sdir)):
            edir = os.path.join(sdir, ex)
            if os.path.isdir(edir) and os.path.exists(os.path.join(edir, "message.txt")):
                yield sig, ex, edir


def write_summary(findings, out_dir, ident, trunk, version):
    """triage.md: every signature, its count and its examples' verdicts."""
    sigs = collections.OrderedDict()
    look = []
    for sig, ex, edir in examples(findings):
        s = sigs.setdefault(sig, {"verdicts": collections.Counter(), "bugs": set(), "examples": 0, "triaged": 0})
        s["examples"] += 1
        try:
            t = read_json(os.path.join(edir, "triage.json"))
        except (OSError, ValueError):
            s["verdicts"]["(not yet)"] += 1
            continue
        s["triaged"] += 1
        s["verdicts"][label(t)] += 1
        s["bugs"].update(t.get("bugs", []))
        if t["verdict"] in UNEXPLAINED:
            look.append((sig, ex, t))
    totals = collections.Counter()
    for s in sigs.values():
        totals.update(s["verdicts"])
    lines = [f"# Pre-triage of {ident.get('run') or out_dir}", "",
             f"Trunk CLI {trunk.checkin}" + (f" ({' '.join(version.split()[:3])})" if version else "") + f", run target {ident.get('target')}; "
             f"{sum(s['examples'] for s in sigs.values())} examples in {len(sigs)} signatures; rewritten {time.strftime('%Y-%m-%d %H:%M:%S UTC', time.gmtime())} "
             "by sqlite-coverage/deploy/pretriage.py (verdicts: its module documentation).", "",
             "Verdicts: " + (", ".join(f"{k} {v}" for k, v in totals.most_common()) or "none"), ""]
    if look:
        lines += ["## Needs a look (new, fixes-only or manual)", ""]
        for sig, ex, t in look:
            lines.append(f"- `{sig}/{ex}`: {label(t)}: {truncate(t.get('reason', ''), 300)}")
        lines.append("")
    lines += ["## Signatures", "", "| Signature | Count | Examples | Verdicts | Bugs |", "|---|---|---|---|---|"]
    for sig, s in sigs.items():
        try:
            count = read_text(os.path.join(findings, sig, "count"))
        except OSError:
            count = "?"
        verdicts = ", ".join(f"{k} ({v})" for k, v in s["verdicts"].most_common())
        lines.append(f"| `{sig}` | {count} | {s['examples']} | {verdicts} | {', '.join(sorted(s['bugs']))} |")
    write_atomic(os.path.join(out_dir, "triage.md"), "\n".join(lines) + "\n")


def triage_dir(findings, out_dir, tri, args, statsd, extra_tags, version):
    first_ex = None
    for sig, ex, edir in examples(findings):
        try:
            first_ex = read_json(os.path.join(edir, "example.json"))
            break
        except (OSError, ValueError):
            continue
    ident = identity(out_dir, first_ex)
    tags = run_tags(ident, extra_tags)
    tri.run_fixes = run_fixes(out_dir)
    counts = collections.Counter()
    done = 0
    started = time.time()
    last_sig = None
    for sig, ex, edir in examples(findings):
        tpath = os.path.join(edir, "triage.json")
        if os.path.exists(tpath) and not args.force:
            continue
        if sig != last_sig:
            tri.r.new_signature()
            last_sig = sig
        if args.budget and time.time() - started > args.budget:
            print(f"pretriage: time budget of {args.budget}s spent in {findings}; the rest next pass")
            break
        t = tri.example(edir)
        exj = {}
        try:
            exj = read_json(os.path.join(edir, "example.json"))
        except (OSError, ValueError):
            pass
        oracle = exj.get("oracle") or sig.split("-", 1)[0]
        profile = exj.get("profile") or "unknown"
        storage = exj.get("storage") or "memory"
        t = dict({"ts": int(time.time()), "signature": sig, "example": ex, "oracle": oracle, "profile": profile, "storage": storage, "label": label(t)}, **t)
        write_atomic(tpath, json.dumps(t, ensure_ascii=False, indent=2) + "\n")
        append_line(os.path.join(out_dir, "triage.jsonl"), dict(t, run=ident.get("run"), path=edir))
        event = {"ts": int(time.time()), "run": ident.get("run"), "kind": "triage", "signature": sig, "example": edir, "verdict": t["verdict"],
                 "bugs": t["bugs"], "label": t["label"], "oracle": oracle, **({"fixed_bugs": tri.run_fixes} if tri.run_fixes else {}), "profile": profile, "storage": storage, "trunk": t["trunk"], "target": t.get("target"),
                 "reason": truncate(t.get("reason"), 600)}
        line = json.dumps(event, ensure_ascii=False, separators=(",", ":"))
        if len(line.encode()) > 3500:
            event["reason"] = truncate(event["reason"], 200)
            event["signature"] = truncate(sig, 200)
        append_line(os.path.join(out_dir, "events.jsonl"), event)
        with open(os.path.join(out_dir, "events.log"), "a", encoding="utf-8") as f:
            f.write(f"{int(time.time())} triage {sig}/{ex}: {t['label']}\n")
        counts[(t["verdict"], tuple(t["bugs"]), oracle, profile, storage)] += 1
        done += 1
        print(f"{sig}/{ex}: {t['label']}  ({truncate(t.get('reason', ''), 160)})", flush=True)
    for (verdict, bugs, oracle, profile, storage), n in counts.items():
        bt = ",".join(f"bug:{b}" for b in bugs) if bugs else "bug:none"
        explained = "false" if verdict in UNEXPLAINED else "true"
        statsd.count("ephyra.sqlite.triage.verdicts", n, f"{tags},verdict:{verdict},{bt},oracle:{oracle},explained:{explained},profile:{profile},storage:{storage}")
    if os.path.isdir(findings):
        write_summary(findings, out_dir, ident, tri.trunk, version)
    summary = collections.Counter()
    for (verdict, bugs, *_), n in counts.items():
        summary["known:" + "+".join(bugs) if verdict == "known" else verdict] += n
    print(f"pretriage: {findings}: {done} examples triaged" + (": " + ", ".join(f"{k} {v}" for k, v in summary.most_common()) if done else ""))
    return done


# ---------------------------------------------------- the nightly check


def run_check(cli_path, repro, timeout):
    with tempfile.TemporaryDirectory(prefix="pretriage-check-") as d, open(repro, "rb") as f:
        try:
            p = subprocess.run([cli_path, "-batch", "-init", os.devnull, ":memory:"], stdin=f,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout, cwd=d)
        except subprocess.TimeoutExpired:
            return None
    return (p.stdout + p.stderr).decode("utf-8", "replace")


def check_bugs(args, registry, root, statsd, extra_tags):
    trunk, patched, status, version = load_clis(args.clis, registry)
    current = os.path.join(root, "current")
    ident = identity(current, None) if os.path.exists(os.path.join(current, "run.json")) else {}
    ident.setdefault("campaign", os.environ.get("CAMPAIGN") or "sqlite/campaign/runner")
    ident.setdefault("work_tag", os.environ.get("TAG") or ident["campaign"])
    tags = run_tags(ident, extra_tags) if ident.get("run") else f"campaign:{ident['campaign']},work_tag:{ident['work_tag']}" + ("," + extra_tags if extra_tags else "")
    out = {"checkin": trunk.checkin, "version": version, "checked": int(time.time()), "bugs": {}}
    for b in open_bugs(registry):
        name = b["name"]
        repro = os.path.join(args.bugs_root, b["repro"])
        got = run_check(trunk.path, repro, args.timeout * 3)
        exp = b.get("expected")
        pst = status.get(name)
        if exp is None:
            state = "unchecked"
        elif got == exp:
            state = "fixed"
        elif pst == "patch_stale":
            state = "stale"
        else:
            state = "still-wrong"
        rec = {"state": state, "trunk": trunk.checkin, "patch": pst, "output": truncate(got, 400)}
        if name in patched and exp is not None:
            pg = run_check(patched[name].path, repro, args.timeout * 3)
            rec["patched_matches_expected"] = pg == exp
            if pg != exp:
                print(f"pretriage: warning: {name}'s patched CLI does not print the expected output (a stale check or a fix that no longer works)")
        out["bugs"][name] = rec
        print(f"{name}: {state} on {trunk.checkin} (patch {pst})")
        kind = {"fixed": "known_bug_fixed", "stale": "patch_stale"}.get(state)
        if kind:
            ev = {"ts": int(time.time()), "run": ident.get("run"), "kind": kind, "bug": name, "checkin": trunk.checkin, "version": version}
            if os.path.isdir(current):
                d = os.path.realpath(current)
                append_line(os.path.join(d, "events.jsonl"), ev)
                with open(os.path.join(d, "events.log"), "a", encoding="utf-8") as f:
                    f.write(f"{int(time.time())} {kind} {name} on SQLite {trunk.checkin}\n")
            append_line(os.path.join(root, "known-bugs-events.jsonl"), ev)
            metric = "ephyra.sqlite.known_bugs.fixed" if state == "fixed" else "ephyra.sqlite.known_bugs.patch_stale"
            statsd.count(metric, 1, f"{tags},bug:{name},checkin:{trunk.checkin}")
    # The retired bugs (fixed upstream, no longer checked), as recorded.
    out["retired"] = {b["name"]: b.get("fixed_in") for b in registry["bugs"] if b.get("status") == "fixed"}
    write_atomic(os.path.join(root, "known-bugs-status.json"), json.dumps(out, indent=2) + "\n")
    return out


def print_expected(args, registry, names):
    _, patched, _, _ = load_clis(args.clis, registry)
    out = {}
    for b in open_bugs(registry):
        if names and b["name"] not in names:
            continue
        if b["name"] in patched:
            out[b["name"]] = run_check(patched[b["name"]].path, os.path.join(args.bugs_root, b["repro"]), args.timeout * 3)
    print(json.dumps(out, indent=2, ensure_ascii=False))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("paths", nargs="*", help="run dirs or findings dirs (default: RUN_ROOT/current)")
    ap.add_argument("--root", default=os.environ.get("RUN_ROOT") or os.path.expanduser("~/fuzz-runs"), help="RUN_ROOT")
    ap.add_argument("--all", action="store_true", help="every run under RUN_ROOT")
    ap.add_argument("--force", action="store_true", help="triage examples that have a triage.json again")
    ap.add_argument("--clis", help="the CLIs dir (default RUN_ROOT/clis/current)")
    ap.add_argument("--registry", default=os.path.join(HERE, "known-bugs.json"))
    ap.add_argument("--bugs-root", default=SQL_DIR, help="what the registry's paths are relative to (the case-studies dir)")
    ap.add_argument("--timeout", type=float, default=20, help="seconds per statement")
    ap.add_argument("--budget", type=float, default=0, help="seconds per findings dir (0: none); the rest waits for the next pass")
    ap.add_argument("--check-bugs", action="store_true", help="run every open bug's check on the trunk CLI")
    ap.add_argument("--expected", nargs="*", metavar="BUG", help="print the bugs' check output on their patched CLIs")
    args = ap.parse_args(argv)
    args.clis = args.clis or os.path.join(args.root, "clis", "current")
    registry = load_registry(args.registry)
    statsd = StatsD(os.environ.get("STATSD"))
    extra_tags = os.environ.get("TAGS", "")
    if args.expected is not None:
        print_expected(args, registry, args.expected)
        return 0
    if args.check_bugs:
        check_bugs(args, registry, args.root, statsd, extra_tags)
        return 0
    trunk, patched, status, version = load_clis(args.clis, registry)
    missing = [n for n, s in status.items() if s not in ("ok", "no-fix")]
    if missing:
        print("pretriage: no patched CLI for " + ", ".join(f"{n} ({status[n]})" for n in missing))
    replayer = Replayer(args.timeout)
    try:
        tri = Triage(trunk, patched, registry, os.path.dirname(os.path.realpath(args.clis)), replayer)
        for findings, out_dir in find_targets(args.paths, args.root, args.all):
            if os.path.isdir(findings):
                triage_dir(findings, out_dir, tri, args, statsd, extra_tags, version)
    finally:
        replayer.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
