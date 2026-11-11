# A call argument's expected type refines the variable for later arguments of the same call

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
  def put(x) do
    Map.put(x, :key, (case x do
      [_ | _] -> throw(:list_clause_ran)
      _ -> :other
    end))
  end
end

IO.inspect(try(do: A.put([1]), catch: (value -> {:thrown, value})))
```

```
{:thrown, :list_clause_ran}
    warning: the following clause will never match:

        [_ | _] ->

    because it attempts to match on the result of:

        x

    which has type:

        dynamic(map())

    └─ repro.exs:4: A.put/1
```

### Expected behavior

No warning. The `[_ | _]` clause runs.

