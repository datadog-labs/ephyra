from typing import ClassVar

class Base:
    x: ClassVar[int] = 0

class Child(Base):
    x = abs(-2)
