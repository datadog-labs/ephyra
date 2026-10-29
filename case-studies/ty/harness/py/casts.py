# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""casts.py <file|-> <seed>: prints the file with `cast(T, e)` wrapped around some expressions.

`typing.cast` returns its argument, so the program means the same. Where ty finds the value already
of type `T`, it reports `redundant-cast`, whose safe fix (`ty check --fix`) replaces the call with
its argument ("Remove the redundant `cast`"); astcmp.py's `--uncast` comparison then checks that
the fixed file is the input with some casts removed and nothing else changed. `T` is a guess from
the expression's shape: a wrong guess is a cast that is not redundant (no fix).
"""
import ast
import random
import sys

path, seed = sys.argv[1], int(sys.argv[2])
src = sys.stdin.read() if path == "-" else open(path, encoding="utf-8").read()
try:
    tree = ast.parse(src)
except SyntaxError:
    sys.stdout.write(src)
    sys.exit(0)
rng = random.Random(seed)
raw = src.encode("utf-8")
line_start = [0]
for l in raw.splitlines(keepends=True):
    line_start.append(line_start[-1] + len(l))


def off(line: int, col: int) -> int:
    return line_start[line - 1] + col


# Expressions never wrapped: type expressions, assignment targets, f-string pieces, starred
# arguments, slices, and everything below them.
skip: set[int] = set()


def skip_tree(n: ast.AST | None) -> None:
    if n is not None:
        for m in ast.walk(n):
            skip.add(id(m))


for n in ast.walk(tree):
    if isinstance(n, ast.Call) and isinstance(n.func, ast.Name):
        skip.add(id(n.func))
    if isinstance(n, ast.arg):
        skip_tree(n.annotation)
    elif isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
        skip_tree(n.returns)
        for t in n.type_params:
            skip_tree(t)
    elif isinstance(n, ast.AnnAssign):
        skip_tree(n.annotation)
        skip_tree(n.target)
    elif isinstance(n, (ast.TypeAlias, ast.Global, ast.Nonlocal, ast.MatchValue, ast.MatchClass, ast.MatchMapping, ast.MatchSingleton)):
        skip_tree(n)
    elif isinstance(n, ast.ClassDef):
        for t in n.type_params:
            skip_tree(t)
    elif isinstance(n, (ast.Assign, ast.Delete)):
        for t in n.targets:
            skip_tree(t)
    elif isinstance(n, (ast.AugAssign, ast.For, ast.AsyncFor, ast.comprehension, ast.NamedExpr)):
        skip_tree(n.target)
    elif isinstance(n, ast.withitem):
        skip_tree(n.optional_vars)
    elif isinstance(n, ast.JoinedStr):
        seg = ast.get_source_segment(src, n) or ""
        # `f"{x=}"` prints the expression's text: wrapping it would change the program.
        if "=}" in seg or "=!" in seg or "=:" in seg or "= }" in seg:
            skip_tree(n)
        for v in n.values:
            if isinstance(v, ast.FormattedValue):
                skip_tree(v.format_spec)
            skip.add(id(v))
    elif isinstance(n, (ast.Starred, ast.Slice)):
        skip.add(id(n))
    elif isinstance(n, ast.Subscript):
        # `X[...]` is often a type expression (`list[int]`, `Literal[1]`): leave its slice.
        skip_tree(n.slice)
    elif isinstance(n, ast.Call) and isinstance(n.func, ast.Name) and n.func.id in ("TypeVar", "NewType", "TypedDict", "NamedTuple", "ParamSpec", "TypeVarTuple", "cast", "isinstance", "issubclass"):
        for a in n.args[1:]:
            skip_tree(a)
        for k in n.keywords:
            skip_tree(k.value)

in_fstring: set[int] = set()
for n in ast.walk(tree):
    if isinstance(n, ast.JoinedStr):
        for m in ast.walk(n):
            in_fstring.add(id(m))


def guess(e: ast.expr) -> str:
    if isinstance(e, ast.Compare) or isinstance(e, ast.UnaryOp) and isinstance(e.op, ast.Not):
        return "bool"
    if isinstance(e, ast.JoinedStr):
        return "str"
    if isinstance(e, ast.Call) and isinstance(e.func, ast.Name):
        f = e.func.id
        if f in ("len", "int", "abs", "sum", "hash", "ord", "round"):
            return "int"
        if f in ("str", "repr", "chr", "format"):
            return "str"
        if f in ("isinstance", "bool", "callable", "issubclass", "all", "any"):
            return "bool"
        if f == "float":
            return "float"
    if isinstance(e, ast.Call) and isinstance(e.func, ast.Attribute) and e.func.attr in ("upper", "lower", "strip", "join", "replace", "format"):
        return "str"
    if isinstance(e, ast.BinOp):
        return rng.choice(["int", "int", "str", "float"])
    if isinstance(e, ast.BoolOp):
        return rng.choice(["bool", "int", "str"])
    if isinstance(e, ast.Constant) and e.value is None:
        return "None"
    return rng.choice(["int", "str", "bool", "float", "object", "list[int]", "int | None", "str | None"])


cands = [
    e
    for e in ast.walk(tree)
    if isinstance(e, ast.expr)
    and id(e) not in skip
    and hasattr(e, "end_lineno")
    and not isinstance(e, (ast.Starred, ast.Slice))
]
rng.shuffle(cands)
# Nodes with richer shapes first (operators need parentheses once unwrapped).
cands.sort(key=lambda e: 0 if isinstance(e, (ast.BinOp, ast.BoolOp, ast.Compare, ast.UnaryOp, ast.IfExp, ast.Await, ast.Yield, ast.YieldFrom, ast.NamedExpr, ast.Lambda)) else rng.randint(1, 2))
want = rng.randint(3, 14)
chosen: list[tuple[int, int, ast.expr]] = []
for e in cands:
    if len(chosen) >= want:
        break
    a, b = off(e.lineno, e.col_offset), off(e.end_lineno, e.end_col_offset)
    # Disjoint or strictly nested (a cast inside a cast: two redundant fixes on one span).
    if any(not (b <= x or a >= y or (x <= a and b <= y and (a, b) != (x, y)) or (a <= x and y <= b and (a, b) != (x, y))) for x, y, _ in chosen):
        continue
    chosen.append((a, b, e))

typing_used = False


def wrap(text: str, e: ast.expr) -> str:
    global typing_used
    t = guess(e)
    if isinstance(e, (ast.Yield, ast.YieldFrom, ast.GeneratorExp)) and not text.startswith("("):
        # Only parenthesized as one argument of several.
        text = f"({text})"
    fn = "cast"
    if rng.random() < 0.15:
        fn = "typing.cast"
        typing_used = True
    k = rng.randrange(12)
    fs = id(e) in in_fstring
    if k == 0:
        return f'{fn}("{t}", {text})' if not fs else f"{fn}({t!r}, {text})"
    if k == 1:
        return f"{fn}(typ={t}, val={text})"
    if k == 2:
        return f"{fn}({t}, val={text})"
    if k == 3:
        return f"{fn}({t}, {text},)"
    if k == 4:
        return f"{fn}({t}, ({text}))"
    if k == 5 and not fs:
        return f"{fn}(\n    {t},\n    {text},\n)"
    if k == 6 and not fs:
        return f"{fn}({t},  # cast\n    {text})"
    if k == 7:
        return f"{fn}({t}, {fn}({t}, {text}))"
    return f"{fn}({t}, {text})"


# Right to left; of nested spans the inner one first. An outer span has grown by its inner
# splices (only the outermost already-spliced spans inside it count).
out = raw
delta: dict[tuple[int, int], int] = {}
for a, b, e in sorted(chosen, key=lambda c: (-c[0], c[1])):
    inner = [k for k in delta if a <= k[0] and k[1] <= b]
    grow = sum(delta.pop(k) for k in inner)
    new = wrap(out[a : b + grow].decode("utf-8"), e).encode("utf-8")
    out = out[:a] + new + out[b + grow :]
    delta[(a, b)] = len(new) - (b - a)

res = out.decode("utf-8")
imp = "from typing import cast\n" + ("import typing\n" if typing_used else "")
lines = res.splitlines(keepends=True)
# After a module docstring and `from __future__` imports.
at = 0
for s in tree.body:
    if isinstance(s, ast.ImportFrom) and s.module == "__future__" or at == 0 and isinstance(s, ast.Expr) and isinstance(s.value, ast.Constant) and isinstance(s.value.value, str) and s is tree.body[0]:
        at = s.end_lineno
    else:
        break
res = "".join(lines[:at]) + imp + "".join(lines[at:])
try:
    ast.parse(res)
except SyntaxError:
    res = src
sys.stdout.write(res)
