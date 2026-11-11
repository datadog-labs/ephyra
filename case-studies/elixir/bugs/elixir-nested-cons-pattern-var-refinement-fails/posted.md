# False "clause will never match" for a list pattern whose tail is a cons pattern with variables (`[y | [_]]`)

### Existing issue

- [x] I have searched existing issues and could not find a duplicate.

### Elixir and Erlang/OTP versions

Erlang/OTP 28 [erts-16.4.0.7] [source] [64-bit] [smp:16:16] [ds:16:16:10] [async-threads:1] [jit:ns]
Elixir 1.21.0-dev (compiled with Erlang/OTP 28)   # main at 3a11ef4
Elixir 1.20.5 (compiled with Erlang/OTP 28)

### Operating system

Ubuntu 24.04.5 LTS (Linux 7.0.0, x86_64)

### Current behavior

### Snippet (`repro.exs`)

```elixir
defmodule A do
  def first(x) do
    case [x, 2] do
      [y | [_]] -> y
    end
  end
end

IO.inspect(A.first(:ok), label: "A.first(:ok) returned")
```

### Result 
```
$ elixir repro.exs
A.first(:ok) returned: :ok
    warning: the following clause will never match:

        [y | [_]] ->

    because it attempts to match on the result of:

        [x, 2]

    which has type:

        dynamic(non_empty_list(term()) and not non_empty_list(integer())) or non_empty_list(integer())

    └─ repro.exs:4: A.first/1
```

### Expected behavior

The flat version of the same pattern (`[y, _]`) gives no warning, their result should be the same?
