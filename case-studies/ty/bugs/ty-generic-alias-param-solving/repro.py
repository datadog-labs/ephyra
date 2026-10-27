class C: ...

type A[T] = list[T]

def f[T](x: A[T], y: T) -> T: return y
def g[T](x: list[T], y: T) -> T: return y

f([C() for s in ["a"] if s == "b"], C())  # error: Expected `A[C]`, found `list[Never]`
g([C() for s in ["a"] if s == "b"], C())  # ok
