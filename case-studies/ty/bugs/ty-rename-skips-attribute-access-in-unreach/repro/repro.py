import sys


class C:
    x: int = 1


def f(c: C) -> int:
    if sys.platform == "win32":
        return c.x
    return c.x
