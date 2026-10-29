# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""The `meta__` oracle's rewriter: equivalent spellings of a check file's annotations.

    meta.py <in-dir> <out-dir> <file>...

For each `<in-dir>/c_<i>.py` writes `<out-dir>/<v>_<i>.py` for each variant `v`:

  a (alias)  annotations (or their parts) become type aliases: `type _Ak = ...` (generic over the
             PEP 695 type parameters they use), `_Ak: TypeAlias = ...`, `_Ak = ...` (implicit) or
             `_Ak = TypeAliasType("_Ak", ...)`, each defined right before the top-level statement
             that uses it (an eager one only when every name in it is bound before that).
  s (spell)  union members reordered and spelled `X | Y`, `Union[X, Y]` or `Optional[X]`;
             builtin generics spelled with their `typing` aliases (`List`, `Dict`, `Type`, ...);
             `Literal[a, b]` split into `Literal[a] | Literal[b]`; `Annotated[X, ...]` wrappers.
  q (quote)  whole annotations or subscript arguments as string annotations.
  m (module) a prefix of the top-level definitions moves into another module (`md_<i>.py`, or the
             package `mp_<i>/` re-exporting its submodule) that the file imports back: by name,
             with `__all__` and `*`, through the package, or as `import md_<i>` with every use
             qualified. Also writes `m_<i>.json` (`style`, `files`).

The typing spec makes each of these the same type ("Type aliases ... are equivalent to the original
type", "Union[X, Y] ... X | Y", unions are unordered, typing.List is an alias of list, a string
annotation is the expression it contains; for `m`, names imported from a module are the objects
defined there), so ty must give the rewritten files the same verdicts.
Every choice is a hash of the annotation's own text, so it survives the shrinker removing other code.
"""
import ast
import builtins
import hashlib
import json
import os
import re
import sys

QUALIFIERS = {"Final", "ClassVar", "Required", "NotRequired", "ReadOnly", "InitVar", "TypeIs", "TypeGuard"}
NO_ALIAS = {"Self", "TypeAlias", "Unpack", "Concatenate", "TypeIs", "TypeGuard", "Final", "ClassVar", "Required", "NotRequired",
            "ReadOnly", "InitVar", "Generic", "Protocol"}
TYPING_ALIAS = {"list": "List", "dict": "Dict", "set": "Set", "frozenset": "FrozenSet", "tuple": "Tuple", "type": "Type"}
COLL_ALIAS = {"deque": "Deque", "defaultdict": "DefaultDict", "OrderedDict": "OrderedDict", "Counter": "Counter",
              "ChainMap": "ChainMap"}
BUILTINS = set(dir(builtins))


def h(*parts) -> int:
    return int.from_bytes(hashlib.sha256("\x00".join(map(str, parts)).encode()).digest()[:8], "little")


def head_name(node):
    """`Name` or `mod.Name` -> "Name"."""
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        return node.attr
    return None


def union_members(node):
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.BitOr):
        return union_members(node.left) + union_members(node.right)
    return [node]


def spell_members(node):
    """Members of a union, flattened through nested `|`, `Union[...]` and `Optional[...]`: spelling a
    nested `Optional[X]` on its own would hide its `None` from the one-`None` filter (`None | None | X`)."""
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.BitOr):
        return spell_members(node.left) + spell_members(node.right)
    if isinstance(node, ast.Subscript) and head_name(node.value) == "Optional":
        return spell_members(node.slice) + [ast.Constant(None)]
    if isinstance(node, ast.Subscript) and head_name(node.value) == "Union":
        return [m for e in (node.slice.elts if isinstance(node.slice, ast.Tuple) else [node.slice]) for m in spell_members(e)]
    return [node]


def has_str(node, in_literal=False):
    """A string constant outside `Literal[...]` / `Annotated` metadata (a forward reference)."""
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return not in_literal
    if isinstance(node, ast.Subscript):
        hn = head_name(node.value)
        if hn == "Literal":
            return False
        if hn == "Annotated":
            first = node.slice.elts[0] if isinstance(node.slice, ast.Tuple) else node.slice
            return has_str(first)
    return any(has_str(c) for c in ast.iter_child_nodes(node))


def names_in(node):
    out = set()
    for n in ast.walk(node):
        if isinstance(n, ast.Name):
            out.add(n.id)
    # `Literal[...]` contents and `Annotated` metadata are values, not names of types; still
    # names (enum members `E.A`) must be bound, so they count.
    return out


class File:
    def __init__(self, src: str):
        self.src = src
        self.lines = src.splitlines(keepends=True)
        self.bstarts = []
        off = 0
        self.b = src.encode()
        for l in self.lines:
            self.bstarts.append(off)
            off += len(l.encode())
        self.tree = ast.parse(src)

    def seg(self, node) -> str:
        s = self.bstarts[node.lineno - 1] + node.col_offset
        e = self.bstarts[node.end_lineno - 1] + node.end_col_offset
        return self.b[s:e].decode()

    def span(self, node):
        return (self.bstarts[node.lineno - 1] + node.col_offset, self.bstarts[node.end_lineno - 1] + node.end_col_offset)


class Ctx:
    """Where an annotation sits: the top-level statement index and the type parameters in scope."""

    def __init__(self, top: int, tparams: list, local_names: set, in_class_body: bool):
        self.top = top
        self.tparams = tparams  # [(name, kind)], kind in {"T", "P", "Ts"}
        self.local_names = local_names
        self.in_class_body = in_class_body


class Rewriter:
    def __init__(self, f: File, variant: str):
        self.f = f
        self.v = variant
        self.edits = []  # (start, end, text)
        self.aliases = {}  # top index -> [alias def lines]
        self.nalias = 0
        self.bound_at = {}  # module-level name -> first top-level statement index
        self.legacy = {}  # module-level legacy TypeVar-likes: name -> kind
        self.occ = {}
        for i, st in enumerate(f.tree.body):
            for n in self.binds(st):
                self.bound_at.setdefault(n, i)
            if isinstance(st, ast.Assign) and isinstance(st.value, ast.Call):
                k = {"TypeVar": "T", "ParamSpec": "P", "TypeVarTuple": "Ts"}.get(head_name(st.value.func) or "")
                if k:
                    for t in st.targets:
                        if isinstance(t, ast.Name):
                            self.legacy[t.id] = k
        self.shadowed = set(self.bound_at)
        # ParamSpecs and TypeVarTuples anywhere (they are not types: never wrapped or quoted).
        self.nont = {n for n, k in self.legacy.items() if k != "T"}
        for n in ast.walk(f.tree):
            if isinstance(n, (ast.ParamSpec, ast.TypeVarTuple)):
                self.nont.add(n.name)

    @staticmethod
    def binds(st):
        if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            return [st.name]
        if isinstance(st, ast.Assign):
            return [n.id for t in st.targets for n in ast.walk(t) if isinstance(n, ast.Name)]
        if isinstance(st, ast.AnnAssign) and isinstance(st.target, ast.Name):
            return [st.target.id]
        if isinstance(st, ast.TypeAlias):
            return [st.name.id]
        if isinstance(st, (ast.Import, ast.ImportFrom)):
            return [(a.asname or a.name).split(".")[0] for a in st.names]
        return []

    def rnd(self, key: str, salt: str, n: int) -> int:
        return h(self.v, key, salt) % n

    # --- walking --------------------------------------------------------------------------------

    def run(self) -> str:
        for i, st in enumerate(self.f.tree.body):
            self.stmt(st, Ctx(i, [], set(), False))
        out = bytearray(self.f.b)
        for s, e, t in sorted(self.edits, reverse=True):
            out[s:e] = t.encode()
        text = out.decode()
        if not self.aliases:
            return text
        # Alias definitions before their top-level statements (their lines in the original).
        lines = text.splitlines(keepends=True)
        # Edits never change the number of lines before a top-level statement's first line?
        # They can (a multi-line annotation is rewritten on one line), so map through the edits.
        shift = []
        for i, st in enumerate(self.f.tree.body):
            first = min([st.lineno] + [d.lineno for d in getattr(st, "decorator_list", [])])
            start = self.f.bstarts[first - 1]
            delta = sum(len(t.encode()) - (e - s) for s, e, t in self.edits if e <= start)
            shift.append(start + delta)
        res = bytearray(text.encode())
        for i in sorted(self.aliases, reverse=True):
            res[shift[i]:shift[i]] = "".join(a + "\n" for a in self.aliases[i]).encode()
        return res.decode()

    def stmt(self, st, ctx: Ctx):
        if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef)):
            tp = ctx.tparams + self.tparams_of(st)
            local = set(ctx.local_names)
            c2 = Ctx(ctx.top, tp, local, False)
            a = st.args
            for arg in a.posonlyargs + a.args + a.kwonlyargs:
                if arg.annotation is not None:
                    self.annotation(arg.annotation, c2)
            for arg in (a.vararg, a.kwarg):
                if arg is not None and arg.annotation is not None and not isinstance(arg.annotation, ast.Starred):
                    if not (isinstance(arg.annotation, ast.Subscript) and head_name(arg.annotation.value) == "Unpack"):
                        self.annotation(arg.annotation, c2)
            if st.returns is not None:
                self.annotation(st.returns, c2)
            self.bounds(st, c2)
            inner = Ctx(ctx.top, tp, local | self.local_binds(st.body), False)
            for s in st.body:
                self.stmt(s, inner)
            return
        if isinstance(st, ast.ClassDef):
            tp = ctx.tparams + self.tparams_of(st)
            self.bounds(st, Ctx(ctx.top, tp, ctx.local_names, False))
            body = Ctx(ctx.top, tp, ctx.local_names | self.local_binds(st.body), True)
            for s in st.body:
                self.stmt(s, body)
            return
        if isinstance(st, ast.AnnAssign):
            self.annotation(st.annotation, ctx)
            return
        if isinstance(st, ast.TypeAlias) and self.v != "a":
            self.annotation(st.value, Ctx(ctx.top, ctx.tparams + self.tparams_of(st), ctx.local_names, False), alias_value=True)
            return
        for fld in ("body", "orelse", "finalbody", "handlers", "cases"):
            for s in getattr(st, fld, None) or []:
                if isinstance(s, (ast.ExceptHandler, ast.match_case)):
                    for s2 in s.body:
                        self.stmt(s2, ctx)
                else:
                    self.stmt(s, ctx)

    @staticmethod
    def tparams_of(node):
        out = []
        for p in getattr(node, "type_params", []) or []:
            k = "T" if isinstance(p, ast.TypeVar) else "P" if isinstance(p, ast.ParamSpec) else "Ts"
            out.append((p.name, k))
        return out

    def local_binds(self, body):
        out = set()
        for s in body:
            for n in ast.walk(s):
                if isinstance(n, ast.Name) and isinstance(n.ctx, ast.Store):
                    out.add(n.id)
                elif isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                    out.add(n.name)
        return out

    def bounds(self, node, ctx):
        """PEP 695 bounds and constraints are lazily evaluated annotations too (quote/spell only)."""
        if self.v == "a":
            return
        for p in getattr(node, "type_params", []) or []:
            b = getattr(p, "bound", None)
            if b is None:
                continue
            if isinstance(b, ast.Tuple):
                for e in b.elts:
                    self.annotation(e, ctx, alias_value=True)
            else:
                self.annotation(b, ctx, alias_value=True)

    # --- annotations ----------------------------------------------------------------------------

    def annotation(self, node, ctx: Ctx, alias_value=False):
        if head_name(node) in ("TypeAlias", "Final", "ClassVar", "InitVar"):
            return  # bare qualifiers (`X: TypeAlias = ...`, `x: Final = 1`)
        # TYZ_META_ONLY=<i>,<j>: rewrite only those annotations (in visit order), to bisect a finding.
        self.ann_idx = getattr(self, "ann_idx", -1) + 1
        only = os.environ.get("TYZ_META_ONLY")
        if only is not None and str(self.ann_idx) not in only.split(","):
            return
        key = self.f.seg(node)
        n = self.occ.get(key, 0)
        self.occ[key] = n + 1
        key = f"{key}#{n}" if n else key
        if self.v == "q":
            new = self.quote_top(node, key, alias_value)
        elif self.v == "s":
            new = self.spell(node, key)
        else:
            new = self.alias_top(node, key, ctx)
        if new is not None and new != self.f.seg(node):
            s, e = self.f.span(node)
            self.edits.append((s, e, new))

    # alias

    def alias_top(self, node, key, ctx):
        # Look through qualifiers: Final[X] -> Final[_A].
        if isinstance(node, ast.Subscript) and head_name(node.value) in QUALIFIERS:
            inner = node.slice
            if isinstance(inner, ast.Tuple):
                return None
            new = self.alias_top(inner, key + "/q", ctx)
            if new is None:
                return None
            return f"{self.f.seg(node.value)}[{new}]"
        if isinstance(node, ast.Subscript) and head_name(node.value) == "Annotated" and isinstance(node.slice, ast.Tuple):
            first = node.slice.elts[0]
            new = self.alias_top(first, key + "/an", ctx)
            if new is None:
                return None
            rest = ", ".join(self.f.seg(e) for e in node.slice.elts[1:])
            return f"{self.f.seg(node.value)}[{new}, {rest}]"
        r = self.rnd(key, "alias", 10)
        if r < 6:
            got = self.make_alias(node, key, ctx)
            if got is not None:
                return got
        # Otherwise alias some parts: subscript arguments (not of Literal/Annotated/Callable lists).
        if isinstance(node, ast.Subscript) and head_name(node.value) not in {"Literal", "Annotated"} | QUALIFIERS:
            elts = node.slice.elts if isinstance(node.slice, ast.Tuple) else [node.slice]
            parts = []
            changed = False
            for j, e in enumerate(elts):
                t = None
                if self.rnd(key, f"part{j}", 2) == 0:
                    t = self.make_alias(e, f"{key}/{j}", ctx)
                parts.append(t if t is not None else self.f.seg(e))
                changed |= t is not None
            if changed:
                sl = ", ".join(parts) if isinstance(node.slice, ast.Tuple) else parts[0]
                if isinstance(node.slice, ast.Tuple) and len(parts) == 1:
                    sl += ","
                return f"{self.f.seg(node.value)}[{sl}]"
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.BitOr):
            ms = union_members(node)
            parts = []
            changed = False
            for j, m in enumerate(ms):
                t = None
                if self.rnd(key, f"um{j}", 2) == 0:
                    t = self.make_alias(m, f"{key}/u{j}", ctx)
                parts.append(t if t is not None else self.f.seg(m))
                changed |= t is not None
            if changed:
                return " | ".join(parts)
        return None

    def make_alias(self, node, key, ctx):
        """`_Ak` (or `_Ak[T, ...]`) for `node`, with its definition queued; None if not possible."""
        if isinstance(node, (ast.List, ast.Tuple, ast.Starred)) or (isinstance(node, ast.Constant) and node.value is Ellipsis):
            return None
        if has_str(node):
            return None
        for n in ast.walk(node):
            if isinstance(n, (ast.Subscript, ast.Name, ast.Attribute)) and head_name(n.value if isinstance(n, ast.Subscript) else n) in NO_ALIAS:
                return None
        names = names_in(node)
        tp = dict(ctx.tparams)
        if isinstance(node, ast.Name) and (node.id in tp or node.id in self.legacy):
            return None  # `type A[T] = T` is legal, but `_A = T` names the TypeVar itself
        used_tp = [p for p, _ in ctx.tparams if p in names]
        # Names must be module-level (or builtins); names bound in a class body or a function
        # (including shadowed type parameters) cannot be seen from a module-level alias.
        legacy_used = []
        for nm in sorted(names):
            if nm in tp:
                if tp[nm] != "T":
                    return None
                continue
            if nm in ctx.local_names:
                return None
            if nm in self.legacy:
                if self.legacy[nm] != "T":
                    return None
                legacy_used.append(nm)
                continue
            if nm not in self.bound_at and nm not in BUILTINS:
                return None
        late = any(self.bound_at.get(nm, -1) >= ctx.top for nm in names if nm not in tp)
        text = self.f.seg(node)
        if "\n" in text:
            return None
        kinds = []
        if not legacy_used:
            kinds.append("type")
        if not used_tp and not late:
            kinds += ["TypeAlias", "implicit"]
            if not legacy_used:
                kinds.append("TypeAliasType")
        if not kinds:
            return None
        kind = kinds[self.rnd(key, "kind", len(kinds))]
        self.nalias += 1
        a = f"_A{self.nalias}"
        params = used_tp if kind == "type" else legacy_used
        if kind == "type":
            d = f"type {a}[{', '.join(params)}] = {text}" if params else f"type {a} = {text}"
        elif kind == "TypeAlias":
            d = f"{a}: _t.TypeAlias = {text}"
        elif kind == "implicit":
            d = f"{a} = {text}"
        else:
            d = f"{a} = _t.TypeAliasType({a!r}, {text})"
        self.aliases.setdefault(ctx.top, []).append(d)
        return f"{a}[{', '.join(params)}]" if params else a

    # spell

    def spell(self, node, key):
        """Equivalent spelling of a type expression, as text."""
        seg = self.f.seg
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.BitOr) or (
            isinstance(node, ast.Subscript) and head_name(node.value) in ("Union", "Optional")
        ):
            ms = spell_members(node)
            parts = [self.spell(m, f"{key}/u{j}") if not (isinstance(m, ast.Constant) and m.value is None) else "None"
                     for j, m in enumerate(ms)]
            # `None | None` is a runtime error: keep one `None`.
            parts = [p for j, p in enumerate(parts) if p != "None" or "None" not in parts[:j]]
            # Unions are unordered: rotate or reverse.
            r = self.rnd(key, "order", 3)
            if r == 1:
                parts = parts[::-1]
            elif r == 2 and len(parts) > 1:
                parts = parts[1:] + parts[:1]
            form = self.rnd(key, "form", 3)
            strs = any(has_str(m) for m in ms)
            if len(parts) == 2 and "None" in parts and form == 2:
                other = [p for p in parts if p != "None"][0]
                return f"_t.Optional[{other}]"
            if form == 0 and not strs and len(parts) > 1:
                return " | ".join(parts)
            return f"_t.Union[{', '.join(parts)}]"
        if isinstance(node, ast.Subscript):
            hn = head_name(node.value)
            base = seg(node.value)
            if hn == "Literal":
                elts = node.slice.elts if isinstance(node.slice, ast.Tuple) else [node.slice]
                flat = []
                for e in elts:
                    if isinstance(e, ast.Subscript) and head_name(e.value) == "Literal":
                        flat += e.slice.elts if isinstance(e.slice, ast.Tuple) else [e.slice]
                    else:
                        flat.append(e)
                if len(flat) > 1 and self.rnd(key, "lit", 2) == 0:
                    parts = [f"{base}[{seg(e)}]" for e in flat]
                    if self.rnd(key, "litorder", 2):
                        parts = parts[::-1]
                    return f"_t.Union[{', '.join(parts)}]"
                return seg(node)
            if hn in ("Annotated", "Unpack", "Concatenate"):
                return seg(node)
            if isinstance(node.value, ast.Name) and hn in TYPING_ALIAS and hn not in self.shadowed and self.rnd(key, "alias", 2) == 0:
                base = f"_t.{TYPING_ALIAS[hn]}"
            elif isinstance(node.value, ast.Attribute) and hn in COLL_ALIAS and seg(node.value.value) == "collections" and self.rnd(key, "alias", 2) == 0:
                base = f"_t.{COLL_ALIAS[hn]}"
            if hn in ("Callable",) and isinstance(node.slice, ast.Tuple) and len(node.slice.elts) == 2:
                ps, ret = node.slice.elts
                if isinstance(ps, ast.List):
                    pt = "[" + ", ".join(self.spell(e, f"{key}/p{j}") for j, e in enumerate(ps.elts)) + "]"
                else:
                    pt = seg(ps)
                return f"{base}[{pt}, {self.spell(ret, key + '/r')}]"
            if isinstance(node.slice, ast.Tuple):
                inner = ", ".join(self.spell_arg(e, f"{key}/{j}") for j, e in enumerate(node.slice.elts))
                if len(node.slice.elts) == 1:
                    inner += ","
                if not node.slice.elts:
                    inner = "()"
            else:
                inner = self.spell_arg(node.slice, key + "/0")
            out = f"{base}[{inner}]"
        elif isinstance(node, ast.Name) and node.id in self.nont:
            return seg(node)
        elif isinstance(node, ast.Name) and node.id in TYPING_ALIAS and node.id not in self.shadowed and self.rnd(key, "alias", 3) == 0:
            out = f"_t.{TYPING_ALIAS[node.id]}"
        else:
            out = seg(node)
        qual = isinstance(node, ast.Subscript) and head_name(node.value) in QUALIFIERS
        if not qual and not (isinstance(node, ast.Constant) and node.value is Ellipsis) and self.rnd(key, "annot", 6) == 0:
            out = f"_t.Annotated[{out}, {self.rnd(key, 'm', 100)}]"
        return out

    def spell_arg(self, e, key):
        if isinstance(e, (ast.Constant, ast.Starred, ast.List, ast.Tuple)):
            return self.f.seg(e)
        if isinstance(e, ast.Name) and e.id in self.nont:
            return self.f.seg(e)
        return self.spell(e, key)

    # quote

    def quote_top(self, node, key, alias_value):
        seg = self.f.seg
        text = seg(node)
        if "\n" in text:
            return None
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            return None
        # ty drops the pydantic metadata of a quoted `Annotated[T, Field(...)]` (alias, frozen,
        # strict): only 1 in 16 of those, so that one bug does not hide the rest.
        if ("Field(" in text or "Strict(" in text) and self.rnd(key, "qpyd", 16) != 0:
            return None
        r = self.rnd(key, "q", 3)
        if r == 0 and not has_str(node):
            return quote(text)
        if r == 1 and isinstance(node, ast.Subscript) and head_name(node.value) not in ("Literal", "Annotated"):
            return self.quote_args(node, key)
        return None

    def quote_args(self, node, key):
        seg = self.f.seg
        elts = node.slice.elts if isinstance(node.slice, ast.Tuple) else [node.slice]
        parts = []
        for j, e in enumerate(elts):
            t = seg(e)
            ok = not isinstance(e, (ast.Starred, ast.List, ast.Tuple)) and not (isinstance(e, ast.Constant) and (e.value is Ellipsis or isinstance(e.value, str)))
            ok = ok and not has_str(e) and not (isinstance(e, ast.Name) and self.legacy.get(e.id, "T") != "T")
            if ok and self.rnd(key, f"qa{j}", 2) == 0:
                t = quote(t)
            elif isinstance(e, ast.Subscript) and head_name(e.value) not in ("Literal", "Annotated"):
                t = self.quote_args(e, f"{key}/{j}")
            elif isinstance(e, ast.List):
                t = "[" + ", ".join(quote(seg(x)) if not has_str(x) and not isinstance(x, (ast.Starred,)) else seg(x) for x in e.elts) + "]"
            parts.append(t)
        sl = ", ".join(parts)
        if isinstance(node.slice, ast.Tuple):
            if len(parts) == 1:
                sl += ","
            if not parts:
                sl = "()"
        return f"{seg(node.value)}[{sl}]"


def quote(text: str) -> str:
    if '"' not in text:
        return f'"{text}"'
    if "'" not in text:
        return f"'{text}'"
    return repr(text)


def paren(t: str) -> str:
    return f"({t})"


class Negator:
    """The `n` (negate) variant: `if c: A else: B` becomes `if <not c>: B else: A`, `a if c else b`
    becomes `b if <not c> else a`, and some conditions become `not <not c>`, where `<not c>` is
    `not (c)` or pushed inwards (De Morgan, `is`/`is not`, `in`/`not in`, `==`/`!=`). Python
    evaluates both programs the same way, so narrowing must give the same types in each branch."""

    def __init__(self, src: str):
        self.src = src
        self.occ = {}

    def key(self, f: File, node) -> str:
        k = f.seg(node)
        n = self.occ.get(k, 0)
        self.occ[k] = n + 1
        return f"{k}#{n}"

    def neg(self, f: File, e, key: str, depth=0) -> str:
        seg = f.seg
        push = h("n", key, depth, "push") % 3 != 0
        if push and isinstance(e, ast.UnaryOp) and isinstance(e.op, ast.Not):
            return paren(seg(e.operand))
        if push and isinstance(e, ast.BoolOp) and depth < 3:
            op = " or " if isinstance(e.op, ast.And) else " and "
            return paren(op.join(self.neg(f, v, f"{key}/{j}", depth + 1) for j, v in enumerate(e.values)))
        if push and isinstance(e, ast.Compare) and len(e.ops) == 1:
            flip = {ast.Is: "is not", ast.IsNot: "is", ast.In: "not in", ast.NotIn: "in", ast.Eq: "!=", ast.NotEq: "=="}
            o = flip.get(type(e.ops[0]))
            if o:
                # (ast spans exclude an operand's own parentheses: `(w := x) is None`)
                return paren(f"{paren(seg(e.left))} {o} {paren(seg(e.comparators[0]))}")
        return f"not {paren(seg(e))}"

    def run(self) -> str:
        f = File(self.src)
        # Pass 1: expressions (ternaries, comprehension filters, conditions kept in place).
        edits = []

        def visit(node, inside):
            if isinstance(node, ast.IfExp) and not inside:
                k = self.key(f, node)
                if h("n", k, "ifexp") % 3 != 0:
                    t = f"({paren(f.seg(node.orelse))} if {self.neg(f, node.test, k)} else {paren(f.seg(node.body))})"
                    edits.append((*f.span(node), t))
                    inside = True
            elif isinstance(node, ast.comprehension):
                for c in [] if inside else node.ifs:
                    k = self.key(f, c)
                    if h("n", k, "compif") % 2 == 0 and "\n" not in f.seg(c):
                        edits.append((*f.span(c), f"not {self.neg(f, c, k)}"))
                for c in [node.target, node.iter]:
                    visit(c, inside)
                return
            elif isinstance(node, ast.While) and not inside:
                k = self.key(f, node.test)
                if h("n", k, "while") % 2 == 0 and "\n" not in f.seg(node.test):
                    edits.append((*f.span(node.test), f"not {self.neg(f, node.test, k)}"))
                    for s in node.body + node.orelse:
                        visit(s, False)
                    return
            for c in ast.iter_child_nodes(node):
                visit(c, inside)

        visit(f.tree, False)
        out = bytearray(f.b)
        for s, e, t in sorted(edits, reverse=True):
            out[s:e] = t.encode()
        text = out.decode()
        # Pass 2: if statements (whole lines move), innermost-free: a swapped statement's blocks
        # are moved verbatim.
        f = File(text)
        lines = f.lines
        repl = []  # (first line, last line, new lines), 1-based inclusive

        def stmts(body):
            for st in body:
                if isinstance(st, ast.If) and self.swap_if(f, st, lines, repl):
                    continue
                for fld in ("body", "orelse", "finalbody"):
                    sub = getattr(st, fld, None)
                    if sub and isinstance(sub, list) and isinstance(sub[0], ast.stmt):
                        stmts(sub)
                for hd in getattr(st, "handlers", []) or []:
                    stmts(hd.body)
                for c in getattr(st, "cases", []) or []:
                    stmts(c.body)

        stmts(f.tree.body)
        for a, b, new in sorted(repl, reverse=True):
            lines[a - 1:b] = new
        return "".join(lines)

    def swap_if(self, f: File, st, lines, repl) -> bool:
        k = self.key(f, st.test)
        if h("n", k, "if") % 3 == 0:
            return False
        body = st.body
        if body[0].lineno == st.lineno or "\n" in f.seg(st.test):
            return False
        header = lines[st.lineno - 1]
        ind = header[: len(header) - len(header.lstrip())]
        if not header.lstrip().startswith("if "):
            return False
        unit = " " * (body[0].col_offset - st.col_offset)
        if not unit:
            return False
        body_lines = lines[body[0].lineno - 1: body[-1].end_lineno]
        if st.orelse:
            o = st.orelse[0]
            first = lines[o.lineno - 1]
            if isinstance(o, ast.If) and first.lstrip().startswith("elif ") and len(first) - len(first.lstrip()) == len(ind):
                chain = lines[o.lineno - 1: st.end_lineno]
                chain[0] = ind + chain[0].lstrip()[2:]  # elif -> if
                else_lines = [unit + l if l.strip() else l for l in chain]
            else:
                else_lines = lines[o.lineno - 1: st.end_lineno]
        else:
            else_lines = [f"{ind}{unit}pass\n"]
        new = [f"{ind}if {self.neg(f, st.test, k)}:\n"] + else_lines + [f"{ind}else:\n"] + body_lines
        if not new[-1].endswith("\n"):
            new[-1] += "\n"
        repl.append((st.lineno, st.end_lineno, new))
        return True


MODULE_STYLES = ["explicit", "star", "package", "qualified", "rename"]


def rebound_names(nodes):
    """Every name bound anywhere in `nodes` (any scope): stores, parameters, type parameters, defs,
    imports, exception and pattern captures."""
    out = set()
    for top in nodes:
        for n in ast.walk(top):
            if isinstance(n, ast.Name) and not isinstance(n.ctx, ast.Load):
                out.add(n.id)
            elif isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                out.add(n.name)
            elif isinstance(n, ast.arg):
                out.add(n.arg)
            elif isinstance(n, (ast.TypeVar, ast.ParamSpec, ast.TypeVarTuple)):
                out.add(n.name)
            elif isinstance(n, (ast.Import, ast.ImportFrom)):
                out.update((a.asname or a.name).split(".")[0] for a in n.names)
            elif isinstance(n, ast.ExceptHandler) and n.name:
                out.add(n.name)
            elif isinstance(n, (ast.MatchAs, ast.MatchStar)) and n.name:
                out.add(n.name)
            elif isinstance(n, ast.MatchMapping) and n.rest:
                out.add(n.rest)
            elif isinstance(n, (ast.Global, ast.Nonlocal)):
                out.update(n.names)
    return out


def deferred_loads(st):
    """(names read when the statement runs, `Name` loads inside function bodies)."""
    eager, deferred = set(), []

    def visit(node, d):
        if isinstance(node, ast.Name):
            if d:
                deferred.append(node)
            else:
                eager.add(node.id)
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)):
            for c in ast.iter_child_nodes(node):
                in_body = c in node.body if isinstance(node.body, list) else c is node.body
                visit(c, d or in_body)
        else:
            for c in ast.iter_child_nodes(node):
                visit(c, d)

    visit(st, False)
    return eager, deferred


def string_ids(node):
    return {w for n in ast.walk(node) if isinstance(n, ast.Constant) and isinstance(n.value, str)
            for w in re.findall(r"[A-Za-z_][A-Za-z0-9_]*", n.value)}


def loaded_names(node):
    """Names a statement reads anywhere, including identifiers inside string constants (forward
    references; over-approximated)."""
    out = set()
    for n in ast.walk(node):
        if isinstance(n, ast.Name):
            out.add(n.id)
        elif isinstance(n, ast.Constant) and isinstance(n.value, str):
            out.update(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", n.value))
    return out


class Splitter:
    """The `m` (module) variant: the file's top-level definitions split at a point into two modules
    that import each other's names. `head`: the definitions before the point move into `md_<i>`
    (a later name read inside a function body there becomes `m_<i>.X`, through an import cycle);
    `tail`: the definitions after it move, `md_<i>` imports what it needs back from `m_<i>`
    (a cycle Python resolves: `m_<i>` imports `md_<i>` only after its own definitions) and the
    trailing calls (`main()`) stay. Styles for the import of `md_<i>`: `from md_<i> import a, b`;
    `__all__` plus `*`; a package `mp_<i>` whose `__init__` re-exports its submodule; `import
    md_<i>` with every use `md_<i>.a`. Python evaluates the pair like the original (nothing at
    module level is mutated, no name is bound on both sides), so ty must give both files the
    original's verdicts."""

    def __init__(self, src: str, i: str, pfx: str = "m"):
        self.f = File(src)
        self.i = i
        self.p = pfx

    def run(self):
        f, body = self.f, self.f.tree.body
        nh = 0
        while nh < len(body) and isinstance(body[nh], (ast.Import, ast.ImportFrom)):
            nh += 1
        nt = len(body)
        while nt > nh and isinstance(body[nt - 1], ast.Expr):
            nt -= 1
        if any(isinstance(n, (ast.Global, ast.Nonlocal)) for n in ast.walk(f.tree)):
            raise ValueError("global statements")
        starts = [min([st.lineno] + [d.lineno for d in getattr(st, "decorator_list", [])]) for st in body]
        text = lambda a, b: "".join(f.lines[starts[a] - 1:(starts[b] - 1 if b < len(body) else len(f.lines))])
        binds = [set(Rewriter.binds(st)) for st in body]
        loads = [loaded_names(st) for st in body]
        dl = [deferred_loads(st) for st in body]  # (eager names, deferred Name nodes)
        rebound = [rebound_names([st]) for st in body]
        strs = [string_ids(st) for st in body]
        me, md = f"{self.p}_{self.i}", f"{self.p}d_{self.i}"
        # The program's own top-level names salt every choice: the generator's prelude is the same
        # in every program, so its text alone would pick the same split everywhere.
        salt = " ".join(sorted(set().union(*binds[nh:nt])))
        if self.p != "m":
            salt += " " + self.p
        cands = []
        for k in range(nh + 1, nt):
            pb, sb = set().union(*binds[nh:k]), set().union(*binds[k:nt])
            if pb & sb or not pb or not sb:
                continue
            cands.append((h("mt", salt, text(k - 1, k)), k, "tail"))
            # head: a prefix definition may read a later name only inside a function body.
            if all(not (loads[j] & sb & (dl[j][0] | rebound[j] | strs[j])) for j in range(nh, k)):
                cands.append((h("m", salt, text(k - 1, k)), k, "head"))
        if not cands:
            raise ValueError("no split point")
        key, k, mode = min(cands)
        key = h("ms", key)
        style = MODULE_STYLES[key % len(MODULE_STYLES)]
        pb, sb = set().union(*binds[nh:k]), set().union(*binds[k:nt])
        header = text(0, nh) if nh else ""

        def qualify(lo, hi, names, mod, suffix=""):
            """Text of statements lo..hi with every load of `names` spelled `mod.X` (or, without a
            `mod`, `X<suffix>`)."""
            base = f.bstarts[starts[lo] - 1]
            b = bytearray(text(lo, hi).encode())
            spans = []
            for j in range(lo, hi):
                for n in ast.walk(body[j]):
                    if isinstance(n, ast.Name) and isinstance(n.ctx, ast.Load) and n.id in names:
                        spans.append(f.span(n))
            for s, e in sorted(spans, reverse=True):
                if mod:
                    b[s - base:s - base] = f"{mod}.".encode()
                else:
                    b[e - base:e - base] = suffix.encode()
            return b.decode()

        tag, need, star_cycle = mode, [], False
        if mode == "head":
            moved, kept, names = (nh, k), (k, len(body)), sorted(pb)
            back = {n for j in range(nh, k) for n in loads[j] & sb}
            if back:
                tag = "headcycle"
            moved_text = (f"import {me}\n" + qualify(nh, k, back, me)) if back else text(nh, k)
            before = ""
        else:
            moved, kept, names = (k, nt), (nt, len(body)), sorted(sb)
            need = sorted(pb & set().union(*loads[k:nt]))
            # Rare (1 in 256): `*` imports both ways (each module with its own `__all__`, so no
            # name is bound twice) leak `Divergent` into the public types of the first module's
            # names (ty 0.17.2+52; cf. ty#113) in most programs, which would hide everything else
            # this variant finds. So does a `*` import without `__all__` of a module that imports
            # names back (it re-binds them to themselves): also only then.
            star_cycle = bool(need) and self.p == "m" and not (key >> 8) % int(os.environ.get("TYZ_META_STAR_CYCLE", "256"))
            if not need:
                back_imp = ""
            elif not star_cycle:
                back_imp = f"from {me} import {', '.join(need)}\n"
            else:
                back_imp = f"from {me} import *\n"
                tag, style = "tailstar", "star"
            moved_text = back_imp + text(k, nt)
            before = text(nh, k)
        files = {}
        star_ok = all(not n.startswith("_") for n in names)

        def dunder_all():
            q = [repr(n) for n in names]
            form = (key >> 4) % 4
            if form == 0 or len(q) < 2:
                return f"__all__ = [{', '.join(q)}]\n"
            half = len(q) // 2
            if form == 1:
                return f"__all__ = ({', '.join(q)},)\n"
            if form == 2:
                return f"__all__ = [{', '.join(q[:half])}]\n__all__ += [{', '.join(q[half:])}]\n"
            return f"__all__: list[str] = [{', '.join(q[:half])}]\n__all__.extend([{', '.join(q[half:])}])\n"

        # The kept part: in `tail` mode the definitions before the point, then the import.
        kept_text = text(*kept)
        if style == "explicit":
            files[f"{md}.py"] = header + moved_text
            imp = f"from {md} import {', '.join(names)}\n"
        elif style == "star":
            plain_ok = star_ok and not need
            all_ = dunder_all() if not plain_ok or (key >> 2) % 2 else ""
            files[f"{md}.py"] = header + all_ + moved_text
            imp = f"from {md} import *\n"
        elif style == "package":
            pkg, sub = f"{self.p}p_{self.i}", f"{self.p}pd_{self.i}"
            files[f"{pkg}/{sub}.py"] = header + dunder_all() + moved_text
            form = (key >> 2) % 3
            if form == 0:
                init = f"from .{sub} import *\nfrom . import {sub}\n__all__ = []\n__all__ += {sub}.__all__\n"
            elif form == 1:
                init = f"from .{sub} import " + ", ".join(f"{n} as {n}" for n in names) + "\n"
            else:
                init = f"from .{sub} import *\nfrom .{sub} import __all__ as __all__\n"
            files[f"{pkg}/__init__.py"] = init
            if form == 1 and (not star_ok or (key >> 6) % 2):
                imp = f"from {pkg} import {', '.join(names)}\n"
            else:
                imp = f"from {pkg} import *\n"
        else:
            files[f"{md}.py"] = header + moved_text
            # Every use in the kept statements is qualified, except names they rebind or name
            # in a string (imported by name as well).
            kj = list(range(*kept)) + (list(range(nh, k)) if mode == "tail" else [])
            blocked = rebound_names([body[j] for j in kj]) | set().union(set(), *(strs[j] for j in kj))
            if mode == "tail":
                # Before the import only function bodies may read them (eager reads were impossible).
                blocked |= set().union(set(), *(dl[j][0] for j in range(nh, k)))
            if style == "rename":
                # A TypeVar-like keeps the name its constructor call gives it.
                blocked |= {n for j in range(*moved) for n in binds[j]
                            if isinstance(body[j], ast.Assign) and isinstance(body[j].value, ast.Call)
                            and head_name(body[j].value.func) in ("TypeVar", "ParamSpec", "TypeVarTuple")}
            qual = set(names) - blocked
            rest = [n for n in names if n not in qual]
            form = (key >> 10) % 3
            if style == "rename":
                # `from md_<i> import X as X_r`, every use `X_r`.
                q, sfx = None, "_r"
                imp = f"from {md} import " + ", ".join(n if n in rest else f"{n} as {n}_r" for n in names) + "\n"
            elif form == 0:
                q, sfx = md, ""
                imp = f"import {md}\n"
            elif form == 1:
                # The module as a submodule of a package with an empty `__init__`.
                del files[f"{md}.py"]
                pkg, sub = f"{self.p}p_{self.i}", f"{self.p}pd_{self.i}"
                files[f"{pkg}/{sub}.py"] = header + moved_text
                files[f"{pkg}/__init__.py"] = ""
                q, sfx = f"{pkg}.{sub}", ""
                imp = f"import {pkg}.{sub}\n"
                md = f"{pkg}.{sub}"
            else:
                q, sfx = "_mq", ""
                imp = f"import {md} as _mq\n"
            if rest and style != "rename":
                imp += f"from {md} import {', '.join(rest)}\n"
            kept_text = qualify(*kept, qual, q, sfx)
            if mode == "tail":
                before = qualify(nh, k, qual, q, sfx)
        own_all = f"__all__ = {sorted(pb)!r}\n" if star_cycle else ""
        main = header + own_all + (before + imp if mode == "tail" else imp) + kept_text
        for t in [main, *files.values()]:
            ast.parse(t)
        return f"{tag}-{style}", main, files


SAFE_BASES = {"object", "Generic", "Exception", "int", "str", "float", "bytes", "list", "dict", "set", "tuple",
              "frozenset", "ABC"}
SPECIAL_BASES = {"Enum", "IntEnum", "StrEnum", "Flag", "IntFlag", "BaseModel", "BaseSettings", "NamedTuple",
                 "TypedDict", "Protocol"}
def class_values(c) -> set:
    """Names a class body reads in values (not annotations, bases or method bodies): those are
    evaluated in order, also in a stub."""
    out = set()
    for st in c.body:
        if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        if isinstance(st, ast.AnnAssign):
            out |= names_in(st.value) if st.value else set()
        elif isinstance(st, ast.ClassDef):
            out |= class_values(st)
        else:
            out |= names_in(st)
    return out


def plain_field(e) -> bool:
    """`field(default=...)` / `field(default_factory=...)` and nothing else: `= ...` in a stub."""
    return isinstance(e, ast.Call) and head_name(e.func) == "field" and not e.args and len(e.keywords) == 1 \
        and e.keywords[0].arg in ("default", "default_factory")


ATTR_CONST = {bool: "bool", int: "int", float: "float", str: "str", bytes: "bytes", type(None): "None"}


def own_nodes(fn):
    """The nodes of a function's own body (not those of nested functions, lambdas or classes)."""
    todo = list(fn.body)
    while todo:
        n = todo.pop()
        yield n
        if not isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda, ast.ClassDef)):
            todo.extend(ast.iter_child_nodes(n))


class Stubber(ast.NodeTransformer):
    """A `.pyi` stub of a module, as stub generators and typeshed write them: function bodies `...`
    (an overload's implementation dropped), instance attributes a method assigns through `self`
    declared in the class body (the type from the annotation, the annotated parameter or the
    constant it is assigned; `Any` otherwise), and, by a hash of the node's text, some annotated
    defaults and annotated plain-class / module values `...` ("The default value of a parameter
    ... may be ..." in stubs). The stub declares what the module defines, so every importer must
    get the same verdicts."""

    def __init__(self, src: str):
        self.src = src
        self.tree = ast.parse(src)
        self.plain = set()  # classes of this module whose annotated values may become `...`
        self.kind = {}  # class name -> "dc" (`@dataclass`), "nt" (NamedTuple), "enum"
        self.need_any = False

    def pick(self, node, salt, n=2):
        return h("st", salt, ast.unparse(node)) % n == 0

    def run(self) -> str:
        body = self.block(self.tree.body, None)
        body = self.reordered(body)
        body = self.versioned(body)
        if self.need_any:
            # After the module's `from __future__` imports.
            at = next((j for j, st in enumerate(body) if not (isinstance(st, ast.ImportFrom) and st.module == "__future__")), len(body))
            body.insert(at, ast.ImportFrom("typing", [ast.alias("Any", "_StubAny")], 0))
        self.tree.body = body
        return ast.unparse(ast.fix_missing_locations(self.tree)) + "\n"

    def reordered(self, body):
        """Typing spec ("Stub files"): "the order of class declarations and type aliases does not
        matter in a stub file". Some runs of consecutive class definitions are reversed (bases and
        annotations then name later classes); not classes with a decorator this module defines."""
        own = {st.name for st in body if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef))}
        movable = lambda st: isinstance(st, ast.ClassDef) and not any(
            names_in(d) & own for d in st.decorator_list) and not any(names_in(k.value) & own for k in st.keywords)
        out, run = [], []
        for st in body + [None]:
            if st is not None and movable(st):
                run.append(st)
                continue
            names = {c.name for c in run}
            if len(run) > 1 and not any(class_values(c) & names for c in run) and self.pick(run[0], "ord", 2):
                run.reverse()
            out.extend(run)
            run = []
            if st is not None:
                out.append(st)
        return out

    def versioned(self, body):
        """Some top-level definitions under `sys.version_info` checks, as typeshed writes them, with a
        decoy definition of the same name in the branch Python 3.13 does not take ("Type checkers
        are expected to understand simple version and platform checks")."""
        out, used = [], False
        defs = [st.name for st in body if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))]
        for st in body:
            if not isinstance(st, (ast.FunctionDef, ast.ClassDef)) or defs.count(st.name) > 1 or not self.pick(st, "ver", 6):
                out.append(st)
                continue
            if isinstance(st, ast.FunctionDef):
                decoy = ast.FunctionDef(st.name, ast.arguments([], [], ast.arg("a", ast.Name("bytes")), [], [], None, []),
                                        [ast.Expr(ast.Constant(...))], [], ast.Name("bytes"), None, [])
            else:
                decoy = ast.ClassDef(st.name, [], [], [ast.Pass()], [], [])
            ast.copy_location(decoy, st)
            v = 8 + h("verv", st.name) % 6
            new = h("verd", st.name) % 2
            cmp = ast.Compare(ast.Attribute(ast.Name("sys"), "version_info"), [ast.GtE() if new else ast.Lt()],
                              [ast.Tuple([ast.Constant(3), ast.Constant(v)])])
            out.append(ast.If(cmp, [st], [decoy]) if new else ast.If(cmp, [decoy], [st]))
            used = True
        if used:
            at = next((j for j, st in enumerate(out) if not (isinstance(st, ast.ImportFrom) and st.module == "__future__")), len(out))
            out.insert(at, ast.Import([ast.alias("sys")]))
        return out

    def block(self, body, cls):
        out = []
        for j, st in enumerate(body):
            if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef)):
                deco = {head_name(d.func if isinstance(d, ast.Call) else d) for d in st.decorator_list}
                if "overload" not in deco and j and any(
                        isinstance(b, (ast.FunctionDef, ast.AsyncFunctionDef)) and b.name == st.name
                        and "overload" in {head_name(d) for d in b.decorator_list} for b in body[:j]):
                    continue
                out.append(self.func(st))
            elif isinstance(st, ast.ClassDef):
                out.append(self.cls(st))
            elif isinstance(st, ast.AnnAssign) and st.value is not None and st.simple \
                    and (cls is None or cls in self.plain or self.kind.get(cls) in ("dc", "nt")) \
                    and (isinstance(st.value, (ast.Constant, ast.Name, ast.UnaryOp)) or self.kind.get(cls) == "dc" and plain_field(st.value)) \
                    and head_name(st.annotation) not in ("Final", "ClassVar", "TypeAlias", "InitVar") and self.pick(st, "v", 3):
                # A module-level declaration may also go without a value ("x: int" in a stub).
                if cls is None and self.pick(st, "nv", 2):
                    st.value, st.simple = None, 1
                else:
                    st.value = ast.Constant(...)
                out.append(st)
            elif isinstance(st, ast.Assign) and self.kind.get(cls) == "enum" and len(st.targets) == 1 \
                    and isinstance(st.targets[0], ast.Name) and not st.targets[0].id.startswith("_") \
                    and isinstance(st.value, ast.Constant) and self.pick(st, "e", 3):
                # Typing spec (enums): "in a stub file, members can be defined using the actual
                # runtime values, or a placeholder of `...` can be used".
                st.value = ast.Constant(...)
                out.append(st)
            elif isinstance(st, ast.If):
                st.body = self.block(st.body, cls) or [ast.Pass()]
                st.orelse = self.block(st.orelse, cls)
                out.append(st)
            else:
                out.append(st)
        return out

    def func(self, fn):
        a = fn.args
        for arg, d in list(zip((a.posonlyargs + a.args)[len(a.posonlyargs + a.args) - len(a.defaults):], range(len(a.defaults)))) + \
                [(arg, None) for arg in a.kwonlyargs]:
            if arg.annotation is None:
                continue
            if d is not None:
                if self.pick(a.defaults[d], "d" + arg.arg, 2):
                    a.defaults[d] = ast.Constant(...)
            else:
                k = a.kwonlyargs.index(arg)
                if a.kw_defaults[k] is not None and self.pick(a.kw_defaults[k], "d" + arg.arg, 2):
                    a.kw_defaults[k] = ast.Constant(...)
        is_gen = any(isinstance(n, (ast.Yield, ast.YieldFrom)) for n in own_nodes(fn))
        fn.body = [ast.Expr(ast.Constant(...))]
        if isinstance(fn, ast.AsyncFunctionDef) and is_gen:
            # Without a `yield` an `async def` is a coroutine function: stubs spell async
            # generators as plain `def`s returning the async iterator.
            fn = ast.copy_location(ast.FunctionDef(fn.name, fn.args, fn.body, fn.decorator_list, fn.returns, fn.type_comment, fn.type_params), fn)
        return fn

    def cls(self, c):
        bases = [head_name(b) for b in c.bases]
        if not c.decorator_list and not c.keywords and all(b in SAFE_BASES or b in self.plain for b in bases):
            self.plain.add(c.name)
        decos = {head_name(d.func if isinstance(d, ast.Call) else d) for d in c.decorator_list}
        if decos == {"dataclass"} and not c.keywords and all(b in SAFE_BASES or b in self.plain for b in bases):
            self.kind[c.name] = "dc"
        elif bases == ["NamedTuple"] and not c.decorator_list:
            self.kind[c.name] = "nt"
        elif bases and all(b == "Enum" or self.kind.get(b) == "enum" for b in bases) and not c.keywords:
            # Not `IntEnum`/`StrEnum`/`Flag`: their `==` and `|` use the values, unknown once they are `...`.
            self.kind[c.name] = "enum"
        declared = {st.target.id for st in c.body if isinstance(st, ast.AnnAssign) and isinstance(st.target, ast.Name)}
        declared |= {t.id for st in c.body if isinstance(st, ast.Assign) for t in st.targets if isinstance(t, ast.Name)}
        declared |= {st.name for st in c.body if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))}
        hoist = {}
        for m in c.body:
            if not isinstance(m, (ast.FunctionDef, ast.AsyncFunctionDef)) or not m.args.args:
                continue
            me = m.args.args[0].arg
            params = {p.arg: p.annotation for p in m.args.posonlyargs + m.args.args + m.args.kwonlyargs if p.annotation}
            for n in ast.walk(m):
                if isinstance(n, ast.AnnAssign):
                    targets, ann, value = [n.target], n.annotation, n.value
                elif isinstance(n, ast.Assign):
                    targets, ann, value = n.targets, None, n.value
                else:
                    continue
                for t in targets:
                    if not (isinstance(t, ast.Attribute) and isinstance(t.value, ast.Name) and t.value.id == me):
                        continue
                    if t.attr in declared:
                        continue
                    if ann is None and isinstance(value, ast.Name) and value.id in params:
                        ann = params[value.id]
                    if ann is None and isinstance(value, ast.Constant) and type(value.value) in ATTR_CONST:
                        ann = ast.Name(ATTR_CONST[type(value.value)])
                    if ann is not None and (names_in(ann) & {tp.name for tp in m.type_params}):
                        ann = None
                    if t.attr not in hoist or hoist[t.attr] is None:
                        hoist[t.attr] = ann
        if hoist and c.name not in self.plain and (set(bases) & SPECIAL_BASES or c.decorator_list or c.keywords
                                                  or any(b not in SAFE_BASES for b in bases)):
            # A field declaration would change a dataclass-like class or a protocol.
            raise ValueError(f"stub: attributes of {c.name}")
        decls = []
        for n, ann in hoist.items():
            if ann is None:
                self.need_any = True
                ann = ast.Name("_StubAny")
            decls.append(ast.AnnAssign(target=ast.Name(n, ast.Store()), annotation=ann, value=None, simple=1))
        c.body = decls + self.block(c.body, c.name)
        return c


def main():
    src_dir, out_dir, files = sys.argv[1], sys.argv[2], sys.argv[3:]
    for arg in files:
        # `c_<i>.py[:<variants>]` (default: all)
        name, _, vs = arg.partition(":")
        with open(f"{src_dir}/{name}") as fh:
            src = fh.read()
        i = name.removeprefix("c_").removesuffix(".py")
        for v in vs or "nasqm":
            try:
                if v == "m":
                    # m_<i>.py, its modules, and m_<i>.json: {"style", "files"} (paths in <out-dir>)
                    style, out, mods = Splitter(src, i).run()
                    for p, t in mods.items():
                        os.makedirs(os.path.dirname(f"{out_dir}/{p}"), exist_ok=True)
                        with open(f"{out_dir}/{p}", "w") as fh:
                            fh.write(t)
                    with open(f"{out_dir}/m_{i}.json", "w") as fh:
                        json.dump({"style": style, "files": list(mods)}, fh)
                if v == "p":
                    # p_<i>.py and the split's modules as `m`, each moved module with a `.pyi` stub
                    # (`<style>` gets `-only` when the `.py` is gone, `-init` for a stubbed package `__init__`)
                    style, out, mods = Splitter(src, i, "p").run()
                    key = h("pstub", *sorted(mods))
                    for p in [p for p in mods if not p.endswith("__init__.py")]:
                        mods[p + "i"] = Stubber(mods[p]).run()
                        ast.parse(mods[p + "i"])
                        if key % 3 == 0:
                            del mods[p]
                            style += "-only"
                    for p in [p for p in mods if p.endswith("__init__.py")]:
                        if (key >> 2) % 2:
                            mods[p + "i"] = mods[p]
                            style += "-init"
                            if (key >> 3) % 2:
                                del mods[p]
                    for p, t in mods.items():
                        os.makedirs(os.path.dirname(f"{out_dir}/{p}"), exist_ok=True)
                        with open(f"{out_dir}/{p}", "w") as fh:
                            fh.write(t)
                    with open(f"{out_dir}/p_{i}.json", "w") as fh:
                        json.dump({"style": style, "files": list(mods)}, fh)
                out = Negator(src).run() if v == "n" else out if v in "mp" else Rewriter(File(src), v).run()
                ast.parse(out)
            except Exception as e:  # noqa: BLE001  (a rewriter miss: the variant is skipped)
                print(f"meta: {name} {v}: {type(e).__name__}: {e}", file=sys.stderr)
                continue
            with open(f"{out_dir}/{v}_{i}.py", "w") as fh:
                fh.write(out)


if __name__ == "__main__":
    main()
