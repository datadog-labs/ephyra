# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""The `lsp__` oracle: `ty server` over stdio, as an editor drives it.

lsp.py <ty> <work> <seed> <in-dir> <name>...

<in-dir>/<name> are check files ty check found clean (with `_rt.py` next to them). For each, F is
the file with non-ASCII comments appended to some lines (positions in UTF-8/16/32 code units
differ), and N is F split into notebook cells at top-level statements. <work>/proj holds F as
`l_<k>.py` and N as `n_<k>.ipynb` on disk. The client opens a broken version of each (edits
applied to F, or to N's cells plus a missing, extra or swapped cell), then sends the inverse
edits as incremental `didChange`s (some typed in chunks; notebook cell inserts, deletes and moves
as structure changes) until the open documents are F and N again. Checked, one JSON line each:

- `cli-<rule>__<doc>`: `ty check` reports an error on F (`text`) or on N (`nb`) on disk;
- `final-<rule>__<doc>`: pulled diagnostics of the restored documents have an error;
- `diff-<rule>__<doc>`: at the open and at one state in the middle of the edits, pulled
  diagnostics (also through `previousResultId`) differ from `ty check` of that state;
- `closed-<rule>__text`: after `didClose`, pulled diagnostics (disk: F) have an error;
- `xpos-<request>__nb`: hover, definition, references, highlights, inlay hints or semantic tokens at
  a position of N's cell differ from the same request at the same position of F;
- `panic`/`exit`/`hang`: the server panicked (stderr or an error response), died, or stopped answering.
"""
import ast
import io
import json
import os
import random
import re
import shutil
import subprocess
import sys
import threading
import queue
import time
import tokenize

TY, WORK, SEED, IND = os.path.abspath(sys.argv[1]), sys.argv[2], int(sys.argv[3]), sys.argv[4]
NAMES = sys.argv[5:]
R = random.Random(SEED)
ENC = R.choice(["utf-16", "utf-16", "utf-8", "utf-32"])
DECOR = ["é", "😀", "ß", "日本", "𝔘", "á", "€", "🐍🐍", "ñ", " x"]
# Known bugs on a cell: documentHighlight and documentSymbol return every cell's results with
# cell-local ranges, prepareCallHierarchy the notebook's URI. Each is compared in 1 session of 40 (on
# its first program) so
# other differences show; TYZ_LSP_FORCE=<request> compares that one only and always (repro).
KNOWN = ("documentHighlight", "documentSymbol", "prepareCallHierarchy")
_draw = {k: R.random() < 1 / 40 for k in KNOWN}
FORCE = os.environ.get("TYZ_LSP_FORCE")
ON = {k: (k == FORCE) if FORCE else _draw[k] for k in KNOWN}
# Diagnostics of closed files (the disk round trip, workspace diagnostics) need diagnosticMode
# "workspace"; half the sessions use it, the others the default (open files only).
WS = R.random() < 0.5
TOML = '[environment]\npython-version = "3.13"\n'
OUT = []
COUNT = {}


def count(k, n=1):
    COUNT[k] = COUNT.get(k, 0) + n


def use(p):
    """Each program's choices come from its own generator (seed and text), so a program shows the
    same session alone as among others (repro)."""
    global R
    if "R" not in p:
        import hashlib
        p["R"] = random.Random(f"{SEED}:{hashlib.sha256(p['src'].encode()).hexdigest()}")
    R = p["R"]
    CUR["p"] = p


CUR = {}


def emit(**kw):
    kw.setdefault("enc", ENC)
    if CUR.get("p") and kw.get("prog") == CUR["p"]["name"]:
        kw.setdefault("crlf", "\r" in CUR["p"].get("T", ""))
    OUT.append(kw)
    print(json.dumps(kw), flush=True)


# ---- text and positions ---------------------------------------------------------------------------


def units(s):
    if ENC == "utf-8":
        return len(s.encode())
    if ENC == "utf-16":
        return len(s.encode("utf-16-le")) // 2
    return len(s)


def lines_of(t):
    """(start offset, content) per line; LSP's line breaks are \\n, \\r\\n and \\r."""
    out, s = [], 0
    for m in re.finditer(r"\r\n|\r|\n", t):
        out.append((s, t[s:m.start()]))
        s = m.end()
    out.append((s, t[s:]))
    return out


def next_line(t, ls, j):
    """The offset after line j's break."""
    return ls[j + 1][0] if j + 1 < len(ls) else len(t)


def to_pos(t, off):
    ls = lines_of(t)
    for k in range(len(ls) - 1, -1, -1):
        if ls[k][0] <= off:
            return {"line": k, "character": units(t[ls[k][0]:off])}
    return {"line": 0, "character": 0}


def char_col(line, ch):
    """The code point column of the position `ch` (code units) in `line`."""
    u = 0
    for k, c in enumerate(line):
        if u >= ch:
            return k
        u += units(c)
    return len(line)


def decorate(src):
    """F: non-ASCII comments at the end of some physical lines (not in strings, not after a
    comment or a backslash)."""
    lines = src.split("\n")
    ends = set()
    try:
        toks = list(tokenize.generate_tokens(io.StringIO(src).readline))
    except (tokenize.TokenError, SyntaxError):
        return src
    has_comment = {t.start[0] for t in toks if t.type == tokenize.COMMENT}
    for t in toks:
        if t.type in (tokenize.NEWLINE, tokenize.NL) and t.start[0] not in has_comment:
            ends.add(t.start[0])
    for ln in sorted(ends):
        if R.random() < 0.12 and ln - 1 < len(lines) and lines[ln - 1].strip():
            lines[ln - 1] += "  # " + "".join(R.choice(DECOR) for _ in range(R.randint(1, 3)))
    out = "\n".join(lines)
    try:
        ast.parse(out)
    except SyntaxError:
        return src
    return out


def split_cells(src):
    """N: F split at top-level statement boundaries into 3..8 cells (some without a trailing newline)."""
    try:
        tree = ast.parse(src)
    except SyntaxError:
        return [src]
    starts = []
    for n in tree.body:
        ln = min([n.lineno] + [d.lineno for d in getattr(n, "decorator_list", [])])
        starts.append(ln - 1)
    lines = src.split("\n")
    # `from __future__` stays in the first cell; the cut points are statement starts.
    cands = sorted(set(s for s in starts[1:] if s > 0))
    k = min(len(cands), R.randint(2, 7))
    cuts = sorted(R.sample(cands, k)) if k else []
    cells, prev = [], 0
    for c in cuts + [len(lines)]:
        cells.append("\n".join(lines[prev:c]))
        prev = c
    out = []
    for k, c in enumerate(cells):
        c = c.rstrip("\n")
        out.append(c if R.random() < 0.3 else c + "\n")
    return [c for c in out if c.strip()]


# Cells that are not F's: markdown, IPython magics (line magics are statements of the notebook's
# source, a cell magic's cell is left out of it), shell escapes.
EXTRAS = [("md", "# Notes é😀\nSome *markdown*, `x = 1`.\n"), ("md", "日本 text"), ("x", "%load_ext autoreload\n%autoreload 2\n"),
          ("x", "%%bash\necho hi 😀\n"), ("x", "!echo ok 🐍\n"), ("x", "%matplotlib inline"), ("x", "%%time\npass\n"),
          ("x", "%env TYZ_Q=1\n%pwd\n")]


def add_extras(cells):
    """N's cells with 0..2 extra cells (never before the first): (texts, kinds)."""
    kinds = ["py"] * len(cells)
    cells = list(cells)
    if R.random() < 0.5:
        for _ in range(R.randint(1, 2)):
            k, t = R.choice(EXTRAS)
            j = R.randint(1, len(cells))
            cells.insert(j, t)
            kinds.insert(j, k)
    return cells, kinds


def nb_json(cells, kinds):
    def cell(k, c, kind):
        if kind == "md":
            return {"cell_type": "markdown", "metadata": {}, "source": c.splitlines(True), "id": f"c{k}"}
        return {"cell_type": "code", "metadata": {}, "source": c.splitlines(True), "outputs": [],
                "execution_count": None, "id": f"c{k}"}
    return json.dumps({
        "cells": [cell(k, c, kind) for k, (c, kind) in enumerate(zip(cells, kinds))],
        "metadata": {"kernelspec": {"name": "python3", "display_name": "Python 3", "language": "python"},
                     "language_info": {"name": "python"}},
        "nbformat": 4, "nbformat_minor": 5})


JUNK = ["x = (\n", "def :\n", "if True\n", "class (:\n", "import nonexistent_mod_q\n", "zz: int = 'a'\n",
        "return\n", "    \n", "print(undefined_name_q)\n", "@\n", "lambda\n", "x: Undefined_q = 1\n"]
TYPES = ["int", "str", "bytes", "float", "bool", "None", "list", "object"]


def mutation(t):
    """An edit (s, e, r) that breaks or changes `t`."""
    all_ls = lines_of(t)
    ls = [x for x in all_ls if x[1].strip()]
    if not ls:
        return (0, 0, R.choice(JUNK))
    k = R.randrange(len(ls))
    s, ln = ls[k]
    m = R.randrange(6)
    if m == 0:  # delete 1..4 lines
        j = min(len(ls) - 1, k + R.randint(0, 3))
        e = next_line(t, all_ls, all_ls.index(ls[j]))
        return (s, e, "")
    if m == 1:  # cut a line short, as if being typed
        c = R.randint(len(ln) - len(ln.lstrip()), len(ln))
        return (s + c, s + len(ln), "")
    if m == 2:  # junk line, indented like this one
        ind = ln[:len(ln) - len(ln.lstrip())]
        return (s, s, ind + R.choice(JUNK))
    if m == 3:  # a builtin type changed
        hits = [x for x in re.finditer(r"\b(" + "|".join(TYPES) + r")\b", ln)]
        if hits:
            h = R.choice(hits)
            return (s + h.start(), s + h.end(), R.choice(TYPES))
    if m == 4:  # an identifier changed
        hits = [x for x in re.finditer(r"\b[A-Za-z_]\w*\b", ln)]
        if hits:
            h = R.choice(hits)
            return (s + h.start(), s + h.end(), h.group() + R.choice(["_", "x", "é", "😀"[:0] + "2"]))
    # a literal or an operator
    hits = [x for x in re.finditer(r"\b\d+\b|==|\bis\b|\bnot\b|\band\b|\bor\b", ln)]
    if hits:
        h = R.choice(hits)
        return (s + h.start(), s + h.end(), R.choice(["0", "'s'", "!=", "is not", "", "None"]))
    return (s + len(ln), s + len(ln), R.choice([" +", "(", ")", ":", " 😀"]))


def broken(t, n):
    """Applies n mutations to t: the broken text and the edits that restore t, in order
    (each (s, e, r) against the text it applies to)."""
    inv = []
    for _ in range(n):
        s, e, r = mutation(t)
        inv.append((s, s + len(r), t[s:e]))
        t = t[:s] + r + t[e:]
    return t, inv[::-1]


def chunked(edits):
    """Splits some inserting edits into chunks typed one after another (and deletes of several
    characters into one per chunk, as backspace does)."""
    out = []
    for s, e, r in edits:
        if len(r) > 3 and R.random() < 0.4:
            if e > s:
                out.append((s, e, ""))
            p = 0
            while p < len(r):
                q = min(len(r), p + R.randint(1, 12))
                if r[q - 1:q + 1] == "\r\n":
                    q += 1  # never between \r and \n
                out.append((s + p, s + p, r[p:q]))
                p = q
        else:
            out.append((s, e, r))
    return out


# ---- the client -----------------------------------------------------------------------------------


class Dead(Exception):
    pass


class Client:
    def __init__(self, root):
        self.p = subprocess.Popen([TY, "server"], cwd=root, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, env={**os.environ, "RUST_BACKTRACE": "0"})
        self.q = {}
        self.cv = threading.Condition()
        self.next = 1
        self.err = []
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=self._stderr, daemon=True).start()

    def _stderr(self):
        for l in self.p.stderr:
            l = l.decode("utf-8", "replace")
            if len(self.err) < 2000:
                self.err.append(l)

    def _read(self):
        f = self.p.stdout
        while True:
            h = {}
            while True:
                l = f.readline()
                if not l:
                    with self.cv:
                        self.q["eof"] = True
                        self.cv.notify_all()
                    return
                l = l.decode().strip()
                if not l:
                    break
                k, v = l.split(":", 1)
                h[k.lower()] = v.strip()
            msg = json.loads(f.read(int(h["content-length"])))
            if "method" in msg and "id" in msg:
                # A request from the server: configuration, registrations, progress.
                res = None
                if msg["method"] == "workspace/configuration":
                    res = [None for _ in msg["params"]["items"]]
                self._send({"jsonrpc": "2.0", "id": msg["id"], "result": res})
            elif "id" in msg:
                with self.cv:
                    self.q[msg["id"]] = msg
                    self.cv.notify_all()

    def _send(self, m):
        b = json.dumps(m).encode()
        try:
            self.p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
            self.p.stdin.flush()
        except (BrokenPipeError, OSError):
            raise Dead()

    def notify(self, method, params):
        self._send({"jsonrpc": "2.0", "method": method, "params": params})

    def request(self, method, params, timeout=30):
        count(method)
        i = self.next
        self.next += 1
        self._send({"jsonrpc": "2.0", "id": i, "method": method, "params": params})
        t0 = time.time()
        with self.cv:
            while i not in self.q:
                if self.q.get("eof"):
                    raise Dead()
                left = timeout - (time.time() - t0)
                if left <= 0:
                    raise TimeoutError(method)
                self.cv.wait(left)
            m = self.q.pop(i)
        if "error" in m:
            e = m["error"]
            msg = e.get("message", "")
            if "panic" in msg.lower():
                raise Panic(msg)
            return {"__error__": msg}
        return m.get("result")

    def close(self):
        try:
            self.request("shutdown", None, timeout=10)
            self.notify("exit", None)
        except (Dead, TimeoutError, Panic):
            pass
        try:
            self.p.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.p.kill()


class Panic(Exception):
    pass


def panic_line(text):
    """'Panicked at <file:line:col> ...' from a server log or an error message, if any."""
    m = re.search(r"panicked at ([^\s:]+\.rs:\d+:\d+):?\s*(.*)", text)
    if m:
        return f"Panicked at {m.group(1)} {m.group(2)[:300]}"
    return None


# ---- diagnostics ----------------------------------------------------------------------------------


def cli(d):
    """`ty check` in d: (file, cell or 0, line, col, severity, rule, message) per diagnostic."""
    p = subprocess.run([TY, "check", "--python-version", "3.13", "--output-format", "concise", "--no-progress",
                        "--project", "."], cwd=d, capture_output=True, text=True, timeout=120)
    out = []
    for l in p.stdout.splitlines():
        m = re.match(r"^(\S+?\.ipynb):cell (\d+):(\d+):(\d+): (\w+)\[([\w-]+)\] (.*)$", l) or \
            re.match(r"^(\S+?\.py):()(\d+):(\d+): (\w+)\[([\w-]+)\] (.*)$", l)
        if m:
            f, c, ln, col, sev, rule, msg = m.groups()
            out.append((f, int(c or 0), int(ln), int(col), sev, rule, msg))
        elif "panicked" in l.lower() or "fatal" in l:
            pl = panic_line(l) or panic_line(p.stderr)
            if pl:
                raise Panic(pl)
    if p.returncode not in (0, 1):
        pl = panic_line(p.stderr + p.stdout)
        if pl:
            raise Panic(pl)
    return out


SEV = {1: "error", 2: "warning"}


def server_diags(c, uri, text, cache):
    """Pulled diagnostics for uri (with the previous result id) as (line, col, severity, rule, message)."""
    prev = cache.get(uri)
    r = c.request("textDocument/diagnostic", {"textDocument": {"uri": uri}, "identifier": "ty",
                                               "previousResultId": prev[0] if prev else None})
    if r is None or "__error__" in r:
        return None
    if r.get("kind") == "unchanged" and prev:
        items = prev[1]
    else:
        items = r.get("items", [])
    cache[uri] = (r.get("resultId"), items)
    ls = [l for _, l in lines_of(text)]
    out = []
    for d in items:
        sev = SEV.get(d.get("severity"))
        if not sev:
            continue
        st = d["range"]["start"]
        ln = st["line"]
        col = char_col(ls[ln], st["character"]) if ln < len(ls) else st["character"]
        # concise output has the message's first line
        out.append((ln + 1, col + 1, sev, str(d.get("code", "")), d["message"].split("\n")[0]))
    return out


def first_diff(a, b):
    """The first diagnostic only one side has: (it, "server" or "cli")."""
    for x in sorted(a):
        if a.count(x) > b.count(x):
            return x, "server"
    for x in sorted(b):
        if b.count(x) > a.count(x):
            return x, "cli"
    return None


# ---- one session ----------------------------------------------------------------------------------


def uri_of(p):
    return "file://" + os.path.abspath(p)


def main():
    proj = os.path.join(WORK, "proj")
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(proj)
    progs = []
    for k, n in enumerate(NAMES):
        p = {"k": k, "name": n, "src": open(os.path.join(IND, n)).read()}
        use(p)
        p["F"] = decorate(p["src"])
        p["N"], p["NK"] = add_extras(split_cells(p["F"]))
        # The text document: F, with CRLF line ends in some programs (the notebook keeps LF)
        p["T"] = p["F"].replace("\n", "\r\n") if R.random() < 0.2 else p["F"]
        progs.append(p)
    for d in [proj]:
        shutil.copy(os.path.join(IND, "_rt.py"), d)
        open(os.path.join(d, "ty.toml"), "w").write(TOML)
    for p in progs:
        use(p)
        open(os.path.join(proj, f"l_{p['k']}.py"), "w").write(p["T"])
        open(os.path.join(proj, f"n_{p['k']}.ipynb"), "w").write(nb_json(p["N"], p["NK"]))
    # The reference: ty check of F and N on disk.
    try:
        ref = cli(proj)
    except Panic as e:
        emit(prog=None, kind="panic", msg=str(e), where="cli")
        return
    for p in progs:
        use(p)
        for f, doc in ((f"l_{p['k']}.py", "text"), (f"n_{p['k']}.ipynb", "nb")):
            errs = [d for d in ref if d[0] == f and d[4] == "error"]
            if errs:
                emit(prog=p["name"], kind=f"cli-{errs[0][5]}__{doc}", msg=f"{f}: {errs[0]}")
                p["bad"] = True

    # The broken states (text: edits on F; notebook: edits on some cells plus a structure change).
    for p in progs:
        use(p)
        B, inv = broken(p["T"], R.randint(1, 3))
        p["B"], p["inv"] = B, chunked(inv)
        cells = list(p["N"])
        nb_edits = []
        for _ in range(R.randint(1, 3)):
            ci = R.randrange(len(cells))
            cb, ci_inv = broken(cells[ci], 1)
            cells[ci] = cb
            nb_edits = [(ci, e) for e in chunked(ci_inv)] + nb_edits
        p["NB"], p["nb_edits"] = cells, nb_edits
        # structure: the opened notebook misses a cell (inserted later), has an extra one (deleted
        # later) or has two cells swapped (moved back later)
        p["struct"] = R.choice(["none", "missing", "extra", "swap"]) if len(cells) > 1 else "none"

    snaps = {}  # state index -> {file: text or cells}
    c = Client(proj)
    findings_before = len(OUT)
    try:
        session(c, proj, progs, snaps)
    except Panic as e:
        emit(prog=None, kind="panic", msg=str(e))
    except Dead:
        pl = panic_line("".join(c.err))
        emit(prog=None, kind="panic" if pl else "exit", msg=pl or "".join(c.err)[-1500:])
    except TimeoutError as e:
        emit(prog=None, kind="hang", msg=f"no answer to {e}")
    finally:
        c.close()
    pl = panic_line("".join(c.err))
    if pl and not any(o["kind"] in ("panic",) for o in OUT[findings_before:]):
        emit(prog=None, kind="panic", msg=pl)


def session(c, proj, progs, snaps):
    caps = {
        "general": {"positionEncodings": [ENC]},
        "textDocument": {"diagnostic": {"dynamicRegistration": False}, "hover": {"contentFormat": ["markdown", "plaintext"]},
                         "synchronization": {"didSave": True}, "inlayHint": {}, "documentSymbol": {"hierarchicalDocumentSymbolSupport": True}, "semanticTokens": {
                             "requests": {"full": True, "range": True}, "tokenTypes": [], "tokenModifiers": [],
                             "formats": ["relative"]}},
        "notebookDocument": {"synchronization": {"dynamicRegistration": False}},
        "workspace": {"configuration": False, "workspaceFolders": True},
    }
    r = c.request("initialize", {"processId": os.getpid(), "rootUri": uri_of(proj), "capabilities": caps,
                                 "workspaceFolders": [{"uri": uri_of(proj), "name": "proj"}],
                                 "initializationOptions": {"diagnosticMode": "workspace"} if WS else {}}, timeout=60)
    c.notify("initialized", {})
    cache = {}
    ok = [p for p in progs if not p.get("bad")]
    # Open every document broken.
    for p in ok:
        use(p)
        k = p["k"]
        p["uri"] = uri_of(os.path.join(proj, f"l_{k}.py"))
        p["nburi"] = uri_of(os.path.join(proj, f"n_{k}.ipynb"))
        p["ver"] = 1
        c.notify("textDocument/didOpen", {"textDocument": {"uri": p["uri"], "languageId": "python", "version": 1,
                                                           "text": p["B"]}})
        p["cur"] = p["B"]
        # Cells: ids stay with their cell through moves.
        ids = [f"{p['nburi'].replace('file://', 'vscode-notebook-cell:')}#X{j}" for j in range(len(p["NB"]))]
        cells = list(zip(ids, p["NB"]))
        p["kind_of"] = dict(zip(ids, p["NK"]))
        p["kind_of"][f"{ids[0]}extra"] = "junk"
        st = p["struct"]
        if st == "missing":
            j = R.randrange(len(cells))
            p["pending"] = (j, cells[j])
            cells = cells[:j] + cells[j + 1:]
        elif st == "extra":
            j = R.randrange(len(cells) + 1)
            cells = cells[:j] + [(f"{ids[0]}extra", R.choice(JUNK))] + cells[j:]
            p["pending"] = (j, None)
        elif st == "swap":
            j = R.randrange(len(cells) - 1)
            cells[j], cells[j + 1] = cells[j + 1], cells[j]
            p["pending"] = (j, "swap")
        p["cells"] = [[u, t, 1] for u, t in cells]
        c.notify("notebookDocument/didOpen", {
            "notebookDocument": {"uri": p["nburi"], "notebookType": "jupyter-notebook", "version": 1,
                                 "cells": [{"kind": kind_num(p, u), "document": u} for u, _, _ in p["cells"]]},
            "cellTextDocuments": [{"uri": u, "languageId": lang(p, u), "version": 1, "text": t} for u, t, _ in p["cells"]]})
        p["nbver"] = 1
    snapshot(c, ok, snaps, 0, cache)
    # Restore: the inverse edits, interleaved with IDE requests at the edit (crash checks); one
    # snapshot in the middle.
    for p in ok:
        use(p)
        n = len(p["inv"])
        mid = R.randrange(n + 1)
        for j, (s, e, r) in enumerate(p["inv"]):
            if j == mid:
                p["mid_text"] = p["cur"]
                p["mid_srv"] = server_diags(c, p["uri"], p["cur"], cache)
            text_edit(c, p, s, e, r)
            if R.random() < 0.25:
                poke(c, p["uri"], p["cur"], s + len(r))
        if mid == n:
            p["mid_text"] = p["cur"]
            p["mid_srv"] = server_diags(c, p["uri"], p["cur"], cache)
        m = len(p["nb_edits"])
        midn = R.randrange(m + 1)
        for j, (ci, (s, e, r)) in enumerate(p["nb_edits"]):
            if j == midn:
                p["mid_cells"] = [t for _, t, _ in p["cells"]]
                p["mid_kinds"] = [p["kind_of"][u] for u, _, _ in p["cells"]]
                p["mid_srv_cells"] = [server_diags(c, u, t, cache) for u, t, _ in p["cells"]]
            cell_edit(c, p, ci, s, e, r)
        if midn == m:
            p["mid_cells"] = [t for _, t, _ in p["cells"]]
            p["mid_kinds"] = [p["kind_of"][u] for u, _, _ in p["cells"]]
            p["mid_srv_cells"] = [server_diags(c, u, t, cache) for u, t, _ in p["cells"]]
        structure_restore(c, p)
    snapshot(c, ok, snaps, 1, cache, mid=True)
    for p in ok:
        use(p)
        if p["cur"] != p["T"] or [t for _, t, _ in p["cells"]] != p["N"]:
            emit(prog=p["name"], kind="harness", msg="restore did not give F/N back")
            p["bad"] = True
    ok = [p for p in ok if not p.get("bad")]
    # Final: no error in the restored documents.
    for p in ok:
        use(p)
        ds = server_diags(c, p["uri"], p["T"], cache) or []
        errs = [d for d in ds if d[2] == "error"]
        if errs:
            emit(prog=p["name"], kind=f"final-{errs[0][3]}__text", msg=str(errs[0]))
        for j, (u, t, _) in enumerate(p["cells"]):
            ds = server_diags(c, u, t, cache) or []
            errs = [d for d in ds if d[2] == "error"]
            if errs:
                emit(prog=p["name"], kind=f"final-{errs[0][3]}__nb", msg=f"cell {j + 1}: {errs[0]}")
                break
    # The same requests on F and on N's cells.
    for p in ok:
        use(p)
        xpos(c, p)
    # Close the text document: diagnostics come from disk (F) again.
    for p in ok:
        use(p)
        c.notify("textDocument/didClose", {"textDocument": {"uri": p["uri"]}})
    # On disk, F becomes the broken text B of the open, then F again (watched-file changes): the
    # first must give the diagnostics ty check gave B, the second none.
    if not WS:
        return
    for p in ok:
        use(p)
        ds = server_diags(c, p["uri"], p["T"], cache)
        errs = [d for d in (ds or []) if d[2] == "error"]
        if errs:
            emit(prog=p["name"], kind=f"closed-{errs[0][3]}__text", msg=str(errs[0]))
            continue
        if "ref0" not in p:
            continue
        path = os.path.join(proj, f"l_{p['k']}.py")
        for text in (p["B"], p["T"]):
            write_disk(path, text)
            c.notify("workspace/didChangeWatchedFiles", {"changes": [{"uri": p["uri"], "type": 2}]})
            ds = server_diags(c, p["uri"], text, cache)
            if ds is None:
                break
            if text is p["B"]:
                dd = first_diff(ds, p["ref0"])
                if dd:
                    emit(prog=p["name"], kind=f"diff-{dd[0][3]}__disk", msg=f"only in {dd[1]}: {dd[0]}")
                    break
            else:
                errs = [d for d in ds if d[2] == "error"]
                if errs:
                    emit(prog=p["name"], kind=f"closed-{errs[0][3]}__disk", msg=str(errs[0]))

    # Workspace diagnostics: nothing anywhere has an error now.
    r = c.request("workspace/diagnostic", {"previousResultIds": []}, timeout=60)
    names = {p["uri"]: p for p in ok}
    for rep in (r or {}).get("items", []) if isinstance(r, dict) else []:
        p = names.get(rep.get("uri"))
        errs = [d for d in rep.get("items") or [] if d.get("severity") == 1]
        if p and errs:
            use(p)
            emit(prog=p["name"], kind=f"wsdiag-{errs[0].get('code')}__text", msg=json.dumps(errs[0])[:600])


MTIME = [time.time_ns() + 10**9]


def write_disk(path, text):
    """Writes text with a later mtime than any before (ty keys file revisions on mtimes)."""
    with open(path, "w", newline="") as f:
        f.write(text)
    MTIME[0] += 10**7
    os.utime(path, ns=(MTIME[0], MTIME[0]))


def text_edit(c, p, s, e, r):
    t = p["cur"]
    rng = {"start": to_pos(t, s), "end": to_pos(t, e)}
    p["cur"] = t[:s] + r + t[e:]
    p["ver"] += 1
    # Some editors send the whole text
    ch = {"text": p["cur"]} if R.random() < 0.05 else {"range": rng, "text": r}
    c.notify("textDocument/didChange", {"textDocument": {"uri": p["uri"], "version": p["ver"]},
                                        "contentChanges": [ch]})


def cell_edit(c, p, ci, s, e, r):
    # ci indexes the cells as broken (before the structure is restored): find it by id.
    u = p["NB_ids"][ci] if "NB_ids" in p else None
    if u is None:
        p["NB_ids"] = [f"{p['nburi'].replace('file://', 'vscode-notebook-cell:')}#X{j}" for j in range(len(p["NB"]))]
        u = p["NB_ids"][ci]
    cell = next((x for x in p["cells"] if x[0] == u), None)
    if cell is None:
        # The missing cell, not open yet: edit its pending text.
        j, (pu, pt) = p["pending"]
        p["pending"] = (j, (pu, pt[:s] + r + pt[e:]))
        return
    t = cell[1]
    rng = {"start": to_pos(t, s), "end": to_pos(t, e)}
    cell[1] = t[:s] + r + t[e:]
    cell[2] += 1
    p["nbver"] += 1
    c.notify("notebookDocument/didChange", {
        "notebookDocument": {"uri": p["nburi"], "version": p["nbver"]},
        "change": {"cells": {"textContent": [{"document": {"uri": u, "version": cell[2]},
                                              "changes": [{"range": rng, "text": r}]}]}}})
    if R.random() < 0.2:
        poke(c, u, cell[1], s + len(r))


def kind_num(p, u):
    return 1 if p["kind_of"][u] == "md" else 2


def lang(p, u):
    return "markdown" if p["kind_of"][u] == "md" else "python"


def structure_restore(c, p):
    st = p["struct"]
    if st == "none":
        return
    j, x = p["pending"]
    p["nbver"] += 1
    if st == "missing":
        u, t = x
        arr = {"start": j, "deleteCount": 0, "cells": [{"kind": kind_num(p, u), "document": u}]}
        p["cells"].insert(j, [u, t, 1])
        did = {"didOpen": [{"uri": u, "languageId": lang(p, u), "version": 1, "text": t}]}
    elif st == "extra":
        u = p["cells"][j][0]
        arr = {"start": j, "deleteCount": 1}
        del p["cells"][j]
        did = {"didClose": [{"uri": u}]}
    else:
        a, b = p["cells"][j], p["cells"][j + 1]
        arr = {"start": j, "deleteCount": 2, "cells": [{"kind": kind_num(p, b[0]), "document": b[0]}, {"kind": kind_num(p, a[0]), "document": a[0]}]}
        p["cells"][j], p["cells"][j + 1] = b, a
        did = {}
    c.notify("notebookDocument/didChange", {"notebookDocument": {"uri": p["nburi"], "version": p["nbver"]},
                                            "change": {"cells": {"structure": {"array": arr, **did}}}})


def poke(c, uri, text, off):
    """Requests an editor sends while typing, at offset off; only panics count."""
    pos = to_pos(text, min(off, len(text)))
    if R.random() < 0.15:
        pos = {"line": pos["line"] + R.choice([0, 0, 1, 5]), "character": pos["character"] + R.randint(1, 40)}
    td = {"textDocument": {"uri": uri}, "position": pos}
    c.request("textDocument/hover", td)
    if R.random() < 0.5:
        c.request("textDocument/completion", td)
    if R.random() < 0.3:
        c.request("textDocument/signatureHelp", td)


def snapshot(c, ok, snaps, idx, cache, mid=False):
    """Pulled diagnostics vs ty check of the same texts (a fresh directory)."""
    d = os.path.join(WORK, f"cli_{idx}")
    os.makedirs(d)
    shutil.copy(os.path.join(IND, "_rt.py"), d)
    open(os.path.join(d, "ty.toml"), "w").write(TOML)
    for p in ok:
        use(p)
        text = p["mid_text"] if mid else p["cur"]
        cells = p["mid_cells"] if mid else [t for _, t, _ in p["cells"]]
        kinds = p["mid_kinds"] if mid else [p["kind_of"][u] for u, _, _ in p["cells"]]
        open(os.path.join(d, f"l_{p['k']}.py"), "w").write(text)
        open(os.path.join(d, f"n_{p['k']}.ipynb"), "w").write(nb_json(cells, kinds))
    ref = cli(d)
    for p in ok:
        use(p)
        if mid:
            text_d, cells_d = p["mid_srv"], p["mid_srv_cells"]
        else:
            text_d = server_diags(c, p["uri"], p["cur"], cache)
            cells_d = [server_diags(c, u, t, cache) for u, t, _ in p["cells"]]
        tag = "mid" if mid else "open"
        f = f"l_{p['k']}.py"
        if text_d is not None:
            theirs = [(x[2], x[3], x[4], x[5], x[6]) for x in ref if x[0] == f]
            if not mid:
                p["ref0"] = theirs
            dd = first_diff(text_d, theirs)
            if dd:
                emit(prog=p["name"], kind=f"diff-{dd[0][3]}__text", msg=f"{tag}: only in {dd[1]}: {dd[0]}")
        f = f"n_{p['k']}.ipynb"
        for j, mine in enumerate(cells_d):
            if mine is None:
                continue
            theirs = [(x[2], x[3], x[4], x[5], x[6]) for x in ref if x[0] == f and x[1] == j + 1]
            dd = first_diff(mine, theirs)
            if dd:
                emit(prog=p["name"], kind=f"diff-{dd[0][3]}__nb", msg=f"{tag}: cell {j + 1}: only in {dd[1]}: {dd[0]}")
                break


def norm_loc(p, loc, starts):
    """A location in F (l_<k>.py) or in one of N's cells, as (cell, line, char) in N's terms;
    other files as they are."""
    u = loc.get("uri") or loc.get("targetUri")
    rg = loc.get("range") or loc.get("targetSelectionRange")
    if u == p["uri"]:
        ln = rg["start"]["line"]
        fi = max(j for j, s in enumerate(starts) if s <= ln)
        ci = p["fidx"][fi]
        return (ci, ln - starts[fi], rg["start"]["character"], rg["end"]["line"] - starts[fi], rg["end"]["character"])
    for j, (cu, _, _) in enumerate(p["cells"]):
        if u == cu:
            return (j, rg["start"]["line"], rg["start"]["character"], rg["end"]["line"], rg["end"]["character"])
    return (u, rg["start"]["line"], rg["start"]["character"], rg["end"]["line"], rg["end"]["character"])


def xpos(c, p):
    # Cells join with a newline: a cell "a\nb" or "a\nb\n" covers 2 lines of F.
    fc = [(j, u, t) for j, (u, t, _) in enumerate(p["cells"]) if p["kind_of"][u] == "py"]
    p["fidx"] = [j for j, _, _ in fc]
    starts, ln = [], 0
    for _, _, t in fc:
        starts.append(ln)
        ln += len((t[:-1] if t.endswith("\n") else t).split("\n"))
    flines = p["F"].split("\n")
    # positions: identifier starts and ends in F
    pos = []
    for fi, (j, _, t) in enumerate(fc):
        for li, l in enumerate(t.split("\n")):
            if starts[fi] + li < len(flines) and flines[starts[fi] + li] != l and l:
                emit(prog=p["name"], kind="harness", msg="cell lines differ from F")
                return
            for m in re.finditer(r"[A-Za-z_]\w*", l):
                if "#" in l[:m.start()]:
                    continue
                pos.append((fi, j, li, units(l[:m.start() + R.randint(0, len(m.group()))])))
    R.shuffle(pos)
    seen = set()
    for fi, j, li, ch in pos[:10]:
        cu = p["cells"][j][0]
        fpos = {"line": starts[fi] + li, "character": ch}
        cpos = {"line": li, "character": ch}
        if "completion" not in seen and R.random() < 0.15:
            # Completion labels and their edits' ranges, not auto-import ones (a notebook ranks and
            # spells those differently, and F is a module the notebook can import)
            ca = c.request("textDocument/completion", {"textDocument": {"uri": p["uri"]}, "position": fpos})
            cb = c.request("textDocument/completion", {"textDocument": {"uri": cu}, "position": cpos})
            def items(r, uri):
                its = r.get("items", []) if isinstance(r, dict) else (r or [])
                out = set()
                for x in its:
                    if " (import " in x["label"] or "." in x["label"] or x.get("additionalTextEdits"):
                        continue
                    te = x.get("textEdit")
                    rg = te and (te.get("range") or te.get("replace"))
                    out.add((x["label"], x.get("kind"), str(norm_loc(p, {"uri": uri, "range": rg}, starts)) if rg else None))
                return out
            ia, ib = items(ca, p["uri"]), items(cb, cu)
            if ia != ib:
                emit(prog=p["name"], kind="xpos-completion__nb",
                     msg=f"cell {j + 1} line {li + 1} char {ch}: text {sorted(ia - ib)[:4]} nb {sorted(ib - ia)[:4]}")
                seen.add("completion")
        reqs = ("hover", "definition", "references", "typeDefinition", "declaration")
        for req in reqs + tuple(k for k in ("documentHighlight", "prepareCallHierarchy") if known_on(p, k)):
            if req in seen:
                continue
            extra = {"context": {"includeDeclaration": True}} if req == "references" else {}
            a = c.request(f"textDocument/{req}", {"textDocument": {"uri": p["uri"]}, "position": fpos, **extra})
            b = c.request(f"textDocument/{req}", {"textDocument": {"uri": cu}, "position": cpos, **extra})
            if req == "hover":
                na = a and (a["contents"], norm_loc(p, {"uri": p["uri"], "range": a["range"]}, starts) if a.get("range") else None)
                nb = b and (b["contents"], norm_loc(p, {"uri": cu, "range": b["range"]}, starts) if b.get("range") else None)
            elif req == "documentHighlight":
                # A cell's highlights are the ones in that cell.
                na = sorted(x for x in (norm_loc(p, {"uri": p["uri"], "range": h["range"]}, starts) for h in (a or [])) if x[0] == j)
                nb = sorted(norm_loc(p, {"uri": cu, "range": h["range"]}, starts) for h in (b or []))
            elif req == "prepareCallHierarchy":
                na = sorted(str((h["name"], norm_loc(p, {"uri": h["uri"], "range": h["selectionRange"]}, starts))) for h in (a or []))
                nb = sorted(str((h["name"], norm_loc(p, {"uri": h["uri"], "range": h["selectionRange"]}, starts))) for h in (b or []))
            else:
                la = a if isinstance(a, list) else ([a] if a else [])
                lb = b if isinstance(b, list) else ([b] if b else [])
                na = sorted(map(str, (norm_loc(p, x, starts) for x in la)))
                nb = sorted(map(str, (norm_loc(p, x, starts) for x in lb)))
            if na != nb:
                emit(prog=p["name"], kind=f"xpos-{req}__nb",
                     msg=f"cell {j + 1} line {li + 1} char {ch} (F line {starts[fi] + li + 1}): text {str(na)[:400]} != nb {str(nb)[:400]}")
                seen.add(req)
    # Inlay hints and semantic tokens of a cell vs the cell's lines of F.
    for fi, (j, cu, t) in enumerate(fc):
        nl = len((t[:-1] if t.endswith("\n") else t).split("\n"))
        last = t.split("\n")[nl - 1] if nl - 1 < len(t.split("\n")) else ""
        crng = {"start": {"line": 0, "character": 0}, "end": {"line": nl - 1, "character": units(last)}}
        frng = {"start": {"line": starts[fi], "character": 0}, "end": {"line": starts[fi] + nl - 1, "character": units(last)}}
        a = c.request("textDocument/inlayHint", {"textDocument": {"uri": p["uri"]}, "range": frng}) or []
        b = c.request("textDocument/inlayHint", {"textDocument": {"uri": cu}, "range": crng}) or []
        def label(h):
            # label parts link to definitions: those locations in N's terms
            if not isinstance(h["label"], list):
                return h["label"]
            return [[x.get("value"), str(norm_loc(p, x["location"], starts)) if x.get("location") else None] for x in h["label"]]
        ha = sorted(json.dumps([h["position"]["line"] - starts[fi], h["position"]["character"], label(h)]) for h in a)
        hb = sorted(json.dumps([h["position"]["line"], h["position"]["character"], label(h)]) for h in b)
        if ha != hb:
            emit(prog=p["name"], kind="xpos-inlayHint__nb", msg=f"cell {j + 1}: text {[x for x in ha if x not in hb][:3]} nb {[x for x in hb if x not in ha][:3]}")
            return
        # Folding ranges: the ones of F inside the cell's lines (not import and comment blocks: a cell
        # boundary splits those).
        a = c.request("textDocument/foldingRange", {"textDocument": {"uri": p["uri"]}}) or []
        b = c.request("textDocument/foldingRange", {"textDocument": {"uri": cu}}) or []
        fa = sorted((x["startLine"] - starts[fi], x["endLine"] - starts[fi], x.get("kind")) for x in a
                    if x.get("kind") not in ("imports", "comment") and starts[fi] <= x["startLine"] and x["endLine"] < starts[fi] + nl)
        fb = sorted((x["startLine"], x["endLine"], x.get("kind")) for x in b if x.get("kind") not in ("imports", "comment"))
        if fa != fb:
            emit(prog=p["name"], kind="xpos-foldingRange__nb", msg=f"cell {j + 1}: text {[x for x in fa if x not in fb][:3]} nb {[x for x in fb if x not in fa][:3]}")
            return
        if known_on(p, "documentSymbol"):
            a = c.request("textDocument/documentSymbol", {"textDocument": {"uri": p["uri"]}}) or []
            b = c.request("textDocument/documentSymbol", {"textDocument": {"uri": cu}}) or []
            sa = sorted(x for x in symbols(p, a, p["uri"], starts) if x[2] == j)
            sb = sorted(symbols(p, b, cu, starts))
            if sa != sb:
                emit(prog=p["name"], kind="xpos-documentSymbol__nb", msg=f"cell {j + 1}: text {[x for x in sa if x not in sb][:3]} nb {[x for x in sb if x not in sa][:3]}")
                return
        a = c.request("textDocument/semanticTokens/range", {"textDocument": {"uri": p["uri"]}, "range": frng}) or {}
        b = c.request("textDocument/semanticTokens/full", {"textDocument": {"uri": cu}}) or {}
        ta, tb = tokens(a.get("data", []), -starts[fi]), tokens(b.get("data", []), 0)
        if ta != tb:
            emit(prog=p["name"], kind="xpos-semanticTokens__nb", msg=f"cell {j + 1}: text {[x for x in ta if x not in tb][:3]} nb {[x for x in tb if x not in ta][:3]}")
            return


def known_on(p, k):
    """A known-bad request is compared on the session's first program only (unless forced)."""
    return ON[k] and (FORCE == k or p["k"] == 0)


def symbols(p, syms, uri, starts, depth=0):
    """Hierarchical document symbols, flattened: (depth, name, cell, line, char, end line, end char)."""
    out = []
    for x in syms:
        out.append((depth, x["name"]) + norm_loc(p, {"uri": uri, "range": x["selectionRange"]}, starts))
        out += symbols(p, x.get("children") or [], uri, starts, depth + 1)
    return out


def tokens(data, dl):
    out, ln, ch = [], 0, 0
    for k in range(0, len(data) - 4, 5):
        d_ln, d_ch, n, ty, mod = data[k:k + 5]
        ln += d_ln
        ch = ch + d_ch if d_ln == 0 else d_ch
        out.append((ln + dl, ch, n, ty, mod))
    return out


if __name__ == "__main__":
    try:
        main()
        print(json.dumps({"stats": COUNT}), flush=True)
    except Exception as e:  # the harness's own failure: reported, never a ty finding
        import traceback
        emit(prog=None, kind="harness", msg=traceback.format_exc()[-1500:])
