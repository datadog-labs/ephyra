# False "incompatible types in binary construction" for a `::bits` pattern variable used as `::binary`

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
  def f(<<x::bits>>), do: <<x::binary>>
end

defmodule G do
  def f(x) when is_bitstring(x), do: <<x::binary>>
end

IO.inspect({A.f("ab"), G.f("ab")})
```

```
{"ab", "ab"}
    warning: incompatible types in binary construction:

        <<x::binary>>

    got type:

        bitstring()

    but expected type:

        binary()

    where "x" was given the type:

        # type: bitstring()
        # from: repro.exs:2:12
        <<x::bitstring>>

    └─ repro.exs:2:30: A.f/1
```

### Expected behavior

No warning, as for `G.f/1`.

