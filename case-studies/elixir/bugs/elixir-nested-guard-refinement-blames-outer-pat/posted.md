# False "pattern will never match" on a def head when a nested fn guard refines a head variable

### Existing issue

- [x] I have searched existing issues and could not find a duplicate.

### Elixir and Erlang/OTP versions

```
Erlang/OTP 29 [erts-17.1.1] [source] [64-bit] [smp:18:18] [ds:18:18:10] [async-threads:1] [jit] [dtrace]

Elixir 1.21.0-dev (b57b70b) (compiled with Erlang/OTP 29)
```

Also on Elixir 1.20.5.

### Operating system

macOS 26.7

### Current behavior

```elixir
defmodule A do
  def f(a = b) do
    _ = elem(a, 0)
    fn x when is_map(b) -> x end
  end
end

IO.inspect(is_function(A.f({1})), label: "is_function(A.f({1}))")
```

```
is_function(A.f({1})): true
    warning: the following pattern will never match:

        a = b

    where "b" was given the type:

        # type: dynamic(map())
        # from: repro.exs:4:15
        is_map(b)

    └─ repro.exs:2:11: A.f/1
```

### Expected behavior

No warning on `def f(a = b)`. It matches `{1}`.

