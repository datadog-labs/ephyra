# Type checker crashes (MatchError in `Of.refine_body_var/5`) when a `for` generator uses a variable bound in `into:` or `reduce:`

### Existing issue

- [x] I have searched existing issues and could not find a duplicate.

### Elixir and Erlang/OTP versions

Erlang/OTP 28 [erts-16.4.0.7] [source] [64-bit] [smp:16:16] [ds:16:16:10] [async-threads:1] [jit:ns]

Elixir 1.21.0-dev (3a11ef4) (compiled with Erlang/OTP 28)   # main
Elixir 1.20.5 (compiled with Erlang/OTP 28)

### Operating system

Ubuntu 24.04.5 LTS (Linux 7.0.0, x86_64)

### Current behavior

### Snippet (`repro.exs`)

```elixir
defmodule A do
  def f(list), do: for(x <- items, into: (items = list; []), do: x)
end

IO.inspect(A.f([1, 2]), label: "A.f([1, 2])")
```

### Result

```
$ elixir repro.exs
# main and v1.20.5:
** (RuntimeError) found error while checking types for A.f/1:

** (MatchError) no match of right hand side value:

    %{
      vars: %{
        0 => %{name: :list, ...}
      },
      ...
    }

The exception happened while checking this code:

def f(list) do
  for x <- items,
      do: x,
      into:
        (
          items = list
          []
        )
end

Please report this bug at: https://github.com/elixir-lang/elixir/issues

    (elixir 1.21.0-dev) lib/module/types/of.ex:114: Module.Types.Of.refine_body_var/5
    (elixir 1.21.0-dev) lib/module/types/helpers.ex:583: Module.Types.Helpers.zip_map_reduce/5
    (elixir 1.21.0-dev) lib/module/types/apply.ex:410: Module.Types.Apply.remote/8
    (elixir 1.21.0-dev) lib/module/types/expr.ex:879: Module.Types.Expr.for_clause/3
    (elixir 1.21.0-dev) lib/enum.ex:2643: Enum."-reduce/3-lists^foldl/2-0-"/3
    (elixir 1.21.0-dev) lib/module/types/expr.ex:676: anonymous fn/6 in Module.Types.Expr.of_expr/5
```

### Expected behavior

The module compiles and runs:

```
$ elixir repro.exs
# expected: A.f([1, 2]): [1, 2]
# main:     ** (RuntimeError) found error while checking types for A.f/1: ** (MatchError) ...
```

