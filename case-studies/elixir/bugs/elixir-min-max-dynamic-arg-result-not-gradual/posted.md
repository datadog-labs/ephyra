# `min/2` with a dynamic argument returns a non-dynamic type

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
  def f(x), do: 1 * min(x, true)
end

IO.inspect(A.f(2), label: "A.f(2)")
```

```
A.f(2): 2
    warning: incompatible types given to Kernel.*/2:

        1 * min(x, true)

    given types:

        (
          integer(),
          -dynamic(not true and not float() and not integer()) or true- or
          dynamic(float() or integer())
        )

    but expected one of:

        #1
        integer(), integer()

        #2
        integer(), float()

        #3
        float(), integer()

        #4
        float(), float()

    └─ repro.exs:2:19: A.f/1
```

### Expected behavior

No warning. `min(x, true)` should be `dynamic()` when `x` is.

