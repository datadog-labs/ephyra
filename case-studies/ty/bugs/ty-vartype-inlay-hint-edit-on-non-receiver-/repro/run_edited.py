class C: ...


def f(c, s: str):
    c.a: int = len(s)


f(C(), "ab")
print("ok")
