from typing import Callable

x: Callable[[str], list[float]] | Callable[[str], bool] = lambda s: [1]
