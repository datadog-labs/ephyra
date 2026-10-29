# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Runs generated programs, each in a forked child, and prints one JSON line per program:
{"file", "status": "ok"|"exc"|"timeout"|"crash", "error", "obs": {k: [types]},
"truth": {"line:col:end_line:end_col": [observed bool values]}, "kind": {key: construct}}.

`truth`: every condition (if/elif/while/assert test, conditional expression, comprehension
`if`, match guard, and the operands of `and`/`or`/`not` and the branches of conditional
expressions inside them) is wrapped in a recorder
of its truth value, keyed by its 1-based line and column in the program's source."""
import ast
import builtins
import json
import os
import signal
import sys
import traceback

TIMEOUT = float(os.environ.get("TYZ_PY_TIMEOUT", "3"))


TRUTH: dict[str, set[bool]] = {}
KIND: dict[str, str] = {}


def _tyz_tr_(k: str, v):
    try:
        TRUTH.setdefault(k, set()).add(bool(v))
    except BaseException:  # noqa: BLE001
        pass
    return v


class _Wrap(ast.NodeTransformer):
    def __init__(self, src: str) -> None:
        self.lines = src.splitlines()

    def key(self, e: ast.expr) -> str:
        """`line:col:end_line:end_col` (1-based columns in characters)."""
        def at(lineno: int, off: int) -> int:
            line = self.lines[lineno - 1] if lineno <= len(self.lines) else ""
            return len(line.encode()[:off].decode(errors="replace")) + 1
        return f"{e.lineno}:{at(e.lineno, e.col_offset)}:{e.end_lineno}:{at(e.end_lineno, e.end_col_offset)}"

    def cond(self, e: ast.expr, kind: str) -> ast.expr:
        if isinstance(e, ast.BoolOp):
            e.values = [self.cond(v, kind + "-boolop") for v in e.values]
        elif isinstance(e, ast.UnaryOp) and isinstance(e.op, ast.Not):
            e.operand = self.cond(e.operand, kind + "-not")
        elif isinstance(e, ast.IfExp):
            # ty judges the branches of a conditional expression used as a condition too.
            e.body = self.cond(e.body, kind + "-branch")
            e.orelse = self.cond(e.orelse, kind + "-branch")
        k = self.key(e)
        KIND.setdefault(k, kind)
        call = ast.Call(ast.Name("_tyz_tr_", ast.Load()), [ast.Constant(k), e], [])
        return ast.copy_location(call, e)

    def test(self, n, kind: str):
        self.generic_visit(n)
        n.test = self.cond(n.test, kind)
        return n

    def visit_If(self, n):
        return self.test(n, "if")

    def visit_While(self, n):
        return self.test(n, "while")

    def visit_IfExp(self, n):
        return self.test(n, "ifexp")

    def visit_Assert(self, n):
        return self.test(n, "assert")

    def visit_comprehension(self, n):
        self.generic_visit(n)
        n.ifs = [self.cond(e, "comp") for e in n.ifs]
        return n

    def visit_match_case(self, n):
        self.generic_visit(n)
        if n.guard is not None:
            n.guard = self.cond(n.guard, "guard")
        return n


def _run(path: str) -> None:
    """`runpy.run_path(path, run_name="__main__")` on the instrumented program."""
    import types
    with open(path) as f:
        src = f.read()
    try:
        tree = ast.fix_missing_locations(_Wrap(src).visit(ast.parse(src, path)))
        code = compile(tree, path, "exec")
    except Exception:  # noqa: BLE001
        code = compile(src, path, "exec")
    builtins._tyz_tr_ = _tyz_tr_  # type: ignore[attr-defined]
    mod = types.ModuleType("__main__")
    mod.__file__ = path
    old = sys.modules.get("__main__")
    sys.modules["__main__"] = mod
    try:
        exec(code, mod.__dict__)
    finally:
        if old is not None:
            sys.modules["__main__"] = old


def run_one(path: str) -> dict:
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.close(r)
        signal.alarm(int(TIMEOUT) + 1)
        sys.setrecursionlimit(2000)
        out: dict = {"file": path}
        try:
            import _rt
            _rt.OBS.clear()
            devnull = open(os.devnull, "w")
            sys.stdout = devnull
            _run(path)
            out["status"] = "ok"
        except BaseException as e:  # noqa: BLE001
            out["status"] = "exc"
            out["error"] = f"{type(e).__name__}: {e}"[:300]
            out["tb"] = traceback.format_exc()[-1200:]
        import _rt
        out["obs"] = {str(k): sorted(v) for k, v in _rt.OBS.items()}
        out["truth"] = {k: sorted(v) for k, v in TRUTH.items()}
        out["kind"] = KIND
        os.write(w, json.dumps(out).encode())
        os.close(w)
        os._exit(0)
    os.close(w)
    chunks = []
    while True:
        b = os.read(r, 1 << 16)
        if not b:
            break
        chunks.append(b)
    os.close(r)
    _, status = os.waitpid(pid, 0)
    if not chunks:
        return {"file": path, "status": "crash" if not os.WIFSIGNALED(status) or os.WTERMSIG(status) != signal.SIGALRM else "timeout", "obs": {}}
    return json.loads(b"".join(chunks))


def main() -> None:
    d = sys.argv[1]
    sys.path.insert(0, d)
    for name in sys.argv[2:]:
        print(json.dumps(run_one(os.path.join(d, name))), flush=True)


if __name__ == "__main__":
    main()
