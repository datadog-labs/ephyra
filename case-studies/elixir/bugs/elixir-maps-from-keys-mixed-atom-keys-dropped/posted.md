# `Map.from_keys/2` with mixed keys drops the atom keys from the inferred type

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
  def f do
    case Map.from_keys([:a, "b"], 1) do
      %{a: value} -> value
      _ -> :no_key_a
    end
  end
end

IO.inspect(A.f(), label: "A.f()")
```

```
A.f(): 1
    warning: the following clause will never match:

        %{a: value} ->

    because it attempts to match on the result of:

        Map.from_keys([:a, "b"], 1)

    which has type:

        %{binary() => integer()} and not empty_map()

    └─ repro.exs:4:7: A.f/0
```

### Expected behavior

No warning. The map has key `:a`.

