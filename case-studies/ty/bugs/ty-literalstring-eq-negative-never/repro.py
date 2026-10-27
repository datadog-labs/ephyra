def f() -> None:
    v = "1"
    if v != "42".upper():  # True at runtime: "1" != "42"
        v + 1              # ty: v is Never, no error; runtime: TypeError


f()
