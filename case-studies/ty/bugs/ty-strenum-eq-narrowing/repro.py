from enum import IntEnum, StrEnum
from typing import reveal_type


class S(StrEnum):
    A = "a"


class I(IntEnum):
    ONE = 1


def eq(x: str) -> None:
    if x == S.A:
        reveal_type(x)


def match(x: str) -> None:
    match x:
        case S.A:
            reveal_type(x)


def contains(x: str) -> None:
    if x in (S.A,):
        reveal_type(x)


def int_eq(x: int) -> None:
    if x == I.ONE:
        reveal_type(x)


eq("a")
match("a")
contains("a")
int_eq(1)
