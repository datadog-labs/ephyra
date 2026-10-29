# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""astcmp.py [--uncast] a1 b1 a2 b2 ...: per pair, prints `b<TAB>parses(a)<TAB>parses(b)<TAB>same AST`.

With `--uncast`, every `cast(T, e)` / `typing.cast(T, e)` (also with `typ=`/`val=` keywords) counts as
`e` on both sides: the redundant-cast fix may remove any of them, and nothing else.
"""
import ast
import sys


class Uncast(ast.NodeTransformer):
    def visit_Call(self, n: ast.Call):
        self.generic_visit(n)
        f = n.func
        if isinstance(f, ast.Name) and f.id == "cast" or isinstance(f, ast.Attribute) and f.attr == "cast" and isinstance(f.value, ast.Name) and f.value.id == "typing":
            val = [k.value for k in n.keywords if k.arg == "val"]
            if len(n.args) == 2 and not n.keywords:
                return n.args[1]
            if len(n.args) == 1 and val:
                return val[0]
            if not n.args and val and len(n.keywords) == 2:
                return val[0]
        return n


uncast = sys.argv[1:2] == ["--uncast"]


def dump(path: str):
    try:
        with open(path, encoding="utf-8-sig") as f:
            t = ast.parse(f.read())
        if uncast:
            t = Uncast().visit(t)
        return ast.dump(t)
    except Exception:  # noqa: BLE001
        return None


args = sys.argv[2:] if uncast else sys.argv[1:]
for a, b in zip(args[::2], args[1::2]):
    da, db = dump(a), dump(b)
    print(f"{b}\t{int(da is not None)}\t{int(db is not None)}\t{int(da is not None and da == db)}")
