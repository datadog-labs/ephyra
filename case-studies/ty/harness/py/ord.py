# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Check-order oracle inputs (src/ord.rs).

usage: ord.py <check-dir> <meta-dir> <out-dir> <i>...

For each program i, writes into <out-dir> (with a ty.toml turning on return type inference):
  o_<i>.py      the check file c_<i>.py with most return annotations dropped (`strip_returns`), then `reveal_type` of every top-level name it binds;
  oi_<i>.py     imports o_<i> (`from o_<i> import ...` and `import o_<i>`) and reveals each name both ways;
  and, when meta.py made a module split m_<i> (not its mutual-`*` shape), the split's files with the
  same return annotations dropped and the same reveals appended to each module (its own top-level names).
Prints one JSON line per program: {"i": i, "units": {"single": [files...], "<style>": [files...]},
"entry": [files...], "defs": [files...]} where `entry` are the importing modules (oi_<i>, m_<i>) and
`defs` the imported ones.
"""
import ast
import hashlib
import json
import os
import shutil
import sys

REVEAL = "import typing as _ty_o\n"
TOML = '[analysis]\nreturn-type-inference = "always"\n'


def h(*parts):
    return int.from_bytes(hashlib.sha256("\x00".join(map(str, parts)).encode()).digest()[:8], "little")


def _trivial(body):
    # Stub-like bodies (protocol and abstract members): an inferred `None` would be wrong for the
    # implementations calls dispatch to, which is the declaration's job, not inference's.
    for s in body:
        if isinstance(s, ast.Expr) and isinstance(s.value, ast.Constant):
            continue
        if isinstance(s, (ast.Pass, ast.Raise)):
            continue
        return False
    return True


ITER = ("__iter__", "__next__", "__aiter__", "__anext__", "__getitem__")


def _own_nodes(fn):
    """The nodes of a function's body, not those of nested functions, classes or lambdas."""
    todo = list(fn.body)
    while todo:
        n = todo.pop()
        yield n
        if not isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef, ast.Lambda)):
            todo.extend(ast.iter_child_nodes(n))


def _generator(fn):
    return isinstance(fn, ast.AsyncFunctionDef) or any(isinstance(n, (ast.Yield, ast.YieldFrom)) for n in _own_nodes(fn))


def _returns_def(fn, defs):
    return any(isinstance(n, ast.Return) and isinstance(n.value, ast.Name) and n.value.id in defs for n in _own_nodes(fn))


def strip_returns(src):
    """Drops the return annotation of most functions and methods (ty infers them under
    `return-type-inference = "always"`). Kept: overloads, stub-like bodies, `TypeIs`/`TypeGuard`
    returns (narrowing), and one function in four (by a hash of its text), so inferred and declared
    returns meet in overrides and calls."""
    try:
        tree = ast.parse(src)
    except SyntaxError:
        return src
    lines = src.splitlines(keepends=True)
    offs = [0]
    for l in lines:
        offs.append(offs[-1] + len(l.encode()))
    data = src.encode()
    cuts = []
    defs = {n.name for n in ast.walk(tree) if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))}
    for n in ast.walk(tree):
        if not isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)) or n.returns is None:
            continue
        r = ast.get_source_segment(src, n.returns) or ""
        decos = [ast.get_source_segment(src, d) or "" for d in n.decorator_list]
        if any("overload" in d for d in decos) or _trivial(n.body) or "TypeIs" in r or "TypeGuard" in r:
            continue
        text = ast.get_source_segment(src, n) or n.name
        if h(text) % 4 == 0:
            continue
        # Two shapes hit one known bug each in most programs that have them; 1 in 8 keeps them:
        # generator and async functions and the iterator protocol's methods (not inferred:
        # `Unknown`; a method of a non-final class: `<inferred> | Unknown`; an `Unknown` iterable in
        # a union is dropped when `enumerate`/`zip` solve their type variable), and functions that
        # return one of the program's functions by name (the inferred function literal is disjoint
        # from the function's own).
        if (_generator(n) or _returns_def(n, defs) or n.name in ITER) and h("rare", text) % 8 != 0:
            continue
        a = offs[n.returns.lineno - 1] + n.returns.col_offset
        b = offs[n.returns.end_lineno - 1] + n.returns.end_col_offset
        k = data.rfind(b"->", 0, a)
        if k < 0:
            continue
        while k > 0 and data[k - 1:k] in (b" ", b"\t"):
            k -= 1
        cuts.append((k, b))
    for k, b in sorted(cuts, reverse=True):
        data = data[:k] + data[b:]
    return data.decode()


def top_names(src):
    try:
        tree = ast.parse(src)
    except SyntaxError:
        return []
    out = []

    def add(n):
        if n not in out and not n.startswith("__"):
            out.append(n)

    def targets(t):
        if isinstance(t, ast.Name):
            add(t.id)
        elif isinstance(t, (ast.Tuple, ast.List)):
            for e in t.elts:
                targets(e)
        elif isinstance(t, ast.Starred):
            targets(t.value)

    for s in tree.body:
        if isinstance(s, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            add(s.name)
        elif isinstance(s, ast.Assign):
            for t in s.targets:
                targets(t)
        elif isinstance(s, ast.AnnAssign) and s.value is not None:
            targets(s.target)
        elif isinstance(s, ast.TypeAlias):
            add(s.name.id)
    return [n for n in out if n not in ("main", "_probe")] + (["main"] if "main" in out else [])


def with_reveals(src, names, prefix=""):
    if not names:
        return src
    if not src.endswith("\n"):
        src += "\n"
    return src + REVEAL + "".join(f"_ty_o.reveal_type({prefix}{n})\n" for n in names)


def main():
    cdir, mdir, out = sys.argv[1], sys.argv[2], sys.argv[3]
    os.makedirs(out, exist_ok=True)
    shutil.copy(os.path.join(cdir, "_rt.py"), os.path.join(out, "_rt.py"))
    with open(os.path.join(out, "ty.toml"), "w") as f:
        f.write(TOML)
    for a in sys.argv[4:]:
        i = int(a)
        rec = {"i": i, "units": {}, "entry": [], "defs": []}
        try:
            src = strip_returns(open(os.path.join(cdir, f"c_{i}.py")).read())
        except OSError:
            continue
        names = top_names(src)
        with open(os.path.join(out, f"o_{i}.py"), "w") as f:
            f.write(with_reveals(src, names))
        imp = f"from o_{i} import " + ", ".join(names) + "\n" if names else ""
        imp += f"import o_{i}\n"
        imp = with_reveals(imp, names)
        imp = with_reveals(imp, names, f"o_{i}.")
        with open(os.path.join(out, f"oi_{i}.py"), "w") as f:
            f.write(imp)
        rec["units"]["single"] = [f"o_{i}.py", f"oi_{i}.py"]
        rec["entry"].append(f"oi_{i}.py")
        rec["defs"].append(f"o_{i}.py")
        try:
            info = json.load(open(os.path.join(mdir, f"m_{i}.json")))
        except (OSError, ValueError):
            info = None
        if info and not info.get("style", "").startswith("tailstar"):
            files = [f"m_{i}.py"] + list(info.get("files", []))
            ok = True
            for p in files:
                try:
                    s = strip_returns(open(os.path.join(mdir, p)).read())
                except OSError:
                    ok = False
                    break
                dst = os.path.join(out, p)
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                with open(dst, "w") as f:
                    f.write(with_reveals(s, top_names(s)))
            if ok:
                style = info["style"]
                rec["units"]["module-" + style.split("-")[0]] = files
                rec["entry"].append(files[0])
                rec["defs"].extend(files[1:])
        print(json.dumps(rec))


main()
