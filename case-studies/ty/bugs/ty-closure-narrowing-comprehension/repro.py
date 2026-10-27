def f(p: int | None) -> None:
    if p is not None:
        def inner() -> list[bool]:
            return [p < 1 for _ in range(2)]
