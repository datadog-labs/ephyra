def first[T](x: list[T]) -> T:
    return x[0]

def ident[S: object](a: S) -> S:
    return a

a: int = ident(first([1]))     # error: `object` is not assignable to `int`
b = ident(first([1]))          # fine, `int`
c: int = max(first([1]), 2)    # error: `SupportsDunderLT[Any] | Literal[2]` is not assignable to `int`
