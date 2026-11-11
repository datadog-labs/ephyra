# A clause head that cannot match is subtracted from the next clause's domain

### Existing issue

- [x] I have searched existing issues and could not find a duplicate.

### Elixir and Erlang/OTP versions

```
Erlang/OTP 29 [erts-17.1.1] [source] [64-bit] [smp:18:18] [ds:18:18:10] [async-threads:1] [jit] [dtrace]

Elixir 1.21.0-dev (b57b70b) (compiled with Erlang/OTP 29)
```

main only; Elixir 1.20.5 is not affected.

### Operating system

macOS 26.7

### Current behavior

```elixir
defmodule A do
  def f(x = y) when is_integer(x) and is_atom(y), do: raise("unreachable")
  def f(_), do: :other
end

defmodule B do
  def g, do: A.f(:a)
end

IO.inspect(B.g(), label: "B.g()")
```

```
B.g(): :other
    warning: the following pattern will never match:

        x = y

    where "y" was given the type:

        # type: dynamic(atom())
        # from: repro.exs:2:39
        is_atom(y)

    └─ repro.exs:2:11: A.f/1

    warning: incompatible types given to A.f/1:

        A.f(:a)

    given types:

        -:a-

    but expected one of:

        not atom()

    └─ repro.exs:7:16: B.g/0
```

### Expected behavior

No warning on `B.g/0`. `def f(_)` accepts `:a`. (The warning on `A.f/1` is correct.)

