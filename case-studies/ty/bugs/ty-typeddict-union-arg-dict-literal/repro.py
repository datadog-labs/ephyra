from typing import TypedDict

class TD(TypedDict):
    a: int

def f(p: TD | dict[str, list[float]]) -> None: ...

f({"x": [1]})
