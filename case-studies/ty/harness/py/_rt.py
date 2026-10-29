# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Runtime side of the tyz probes: `_probe(k, v)` records the precise type of `v`
as a type expression (over `_t` = typing) that ty can check `TypeOf[...]` against."""
import enum
import types

OBS: dict[int, set[str]] = {}

# A value whose type cannot be written as a checkable type expression is "?" (no assertion).
def _nominal(t: type) -> str:
    tp = getattr(t, "__type_params__", ())
    if tp:
        return f"{t.__qualname__}[{', '.join('_t.Any' for _ in tp)}]"
    return t.__qualname__


_CLASSES: list[type] | None = None
_BUILTINS = (int, str, float, bytes, bool, list, tuple, dict, set, frozenset)


def _top(t: type) -> str:
    """Every specialization of `t`: `Top[C[Any]]` for a generic class (a bare `C` is
    `C[Unknown]`, whose negation is gradual), else the class name."""
    tp = getattr(t, "__type_params__", ())
    if not tp:
        return t.__qualname__
    import typing
    args = ", ".join("*tuple[_t.Any, ...]" if isinstance(p, typing.TypeVarTuple) else "_t.Any" for p in tp)
    return f"_tx.Top[{t.__qualname__}[{args}]]"


def _program_classes() -> list[type]:
    """Classes bound at module level in the running program, generic ones included
    (ty narrows `C0[int] | float` to `float & ~C0` on `p != 2`). (Walking
    `__subclasses__` would also find the class objects `@dataclass(slots=True)`
    replaces.)"""
    global _CLASSES
    if _CLASSES is None:
        import sys
        main = sys.modules.get("__main__")
        seen: list[type] = []
        for name, c in vars(main).items() if main else ():
            # (Not pydantic's parametrized generic models, which it binds as `C[int]`.)
            if isinstance(c, type) and c.__module__ == "__main__" and c.__qualname__ == name and name.isidentifier():
                if not isinstance(c, enum.EnumMeta):
                    seen.append(c)
        _CLASSES = seen
    return _CLASSES


def _exact(v: object, nominal: str) -> str:
    """The value's type with `Not[X]` for every program or builtin class `X` it is
    not an instance of. A class name alone also stands for unseen subclasses, which
    can inherit from anything, so ty rightly refuses it against negative narrowing
    (`P & ~C`); the intersection describes what the runtime value actually is."""
    nots = []
    for c in list(_BUILTINS) + _program_classes():
        try:
            if isinstance(v, c):
                continue
        except TypeError:
            continue  # a protocol that is not runtime-checkable, a TypedDict
        nots.append(f"_tx.Not[{_top(c)}]")
    # A value that is truthy now is not always falsy (and the reverse), which is
    # what truthiness narrowing (`C & ~AlwaysFalsy`) relies on.
    try:
        nots.append("_tx.Not[_tx.AlwaysFalsy]" if bool(v) else "_tx.Not[_tx.AlwaysTruthy]")
    except Exception:
        pass
    if not nots:
        return nominal
    return f"_tx.Intersection[{nominal}, {', '.join(nots)}]"


def _render(v: object, depth: int = 0) -> str:
    if depth > 5:
        return "?"
    if v is None:
        return "None"
    t = type(v)
    if t is bool or t is int:
        return f"_t.Literal[{v!r}]"
    if t is str:
        # Always a literal: ty narrows `str` on `!=`/`not in` to `str & ~Literal[...]`, which a
        # plain `str` rendering is not assignable to (false sound__assign). ty keeps string literals
        # up to 4096 UTF-8 bytes (MAX_STRING_LITERAL_SIZE); beyond that, or for lone surrogates,
        # no assertion. ascii() escapes non-ASCII so the check file stays plain.
        try:
            n = len(v.encode("utf-8"))
        except UnicodeEncodeError:
            return "?"
        return f"_t.Literal[{ascii(v)}]" if n <= 4096 else "?"
    if t is bytes:
        return f"_t.Literal[{v!r}]" if len(v) <= 4096 else "?"
    if t is float:
        # A `float` annotation means `int | float`.
        # Not[...] for the program classes: a float subclass could also be one.
        return _exact(v, "TypeOf[0.5]")
    if isinstance(v, enum.Enum):
        if t.__module__ == "__main__" and t.__members__.get(v.name) is v:
            return f"_t.Literal[{t.__qualname__}.{v.name}]"
        return "?"
    if t is tuple:
        parts = [_render(x, depth + 1) for x in v]
        if "?" in parts:
            return "?"
        return _exact(v, f"tuple[{', '.join(parts)}]" if parts else "tuple[()]")
    if t is list:
        return _exact(v, "list[_t.Any]")
    if t is dict:
        # Could be a TypedDict, which a plain dict type is not assignable to.
        return "?"
    if t is set:
        return "set[_t.Any]"
    if t is frozenset:
        return "frozenset[_t.Any]"
    if isinstance(v, (types.FunctionType, types.MethodType, types.BuiltinFunctionType)):
        # ty types a function (or lambda) by its own signature, which `Callable[..., Any]`
        # is not assignable to: name a module-level function exactly, else no assertion.
        import sys
        main = sys.modules.get("__main__")
        if isinstance(v, types.FunctionType) and main is not None and vars(main).get(v.__qualname__) is v:
            return f"TypeOf[{v.__qualname__}]"
        return "?"
    if isinstance(v, type):
        if v.__module__ == "__main__":
            # The class object itself: its class-literal type is exact.
            if "<locals>" not in v.__qualname__:
                # Truthy now, so not always falsy (truthiness narrowing relies on it).
                return f"_tx.Intersection[TypeOf[{v.__qualname__}], _tx.Not[_tx.AlwaysFalsy]]"
            return f"type[{_nominal(v)}]"
        if v.__module__ == "builtins" and v.__name__ in ("int", "str", "bool", "float", "bytes", "list", "dict", "tuple", "object"):
            return f"type[{v.__name__}]"
        return "?"
    if t.__module__ == "__main__" and "<locals>" not in t.__qualname__:
        return _exact(v, _nominal(t))
    return "?"


def _probe(k: int, v: object) -> None:
    OBS.setdefault(k, set()).add(_render(v))
