def f(a): ...
def g(b): ...

def h(flag: bool):
    (f if flag else g)(1)

h(False)
