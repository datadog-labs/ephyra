from typing import final


@final
class C[T]:
    def meth(self) -> int:
        return 1


def f(c: C[int]) -> int:
    return c.meth()


print(f(C()))
