# Guard on a `<<x::bitstring>>` clause is ignored when typing the next clause

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
  def f(<<x::bitstring>>) when is_binary(x), do: :binary
  def f(_), do: :other

  def g, do: f(<<1::1>>) == :other
end

IO.inspect(A.g(), label: "A.g()")
```

```
A.g(): true
    warning: comparison between distinct types found:

        f(<<1::integer-size(1)>>) == :other

    given types:

        dynamic(:binary) == :other

    └─ repro.exs:5:26: A.g/0
```

### Expected behavior

No warning. `f(<<1::1>>)` returns `:other`.

