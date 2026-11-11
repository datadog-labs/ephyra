defmodule A do
  def f(x), do: 1 * min(x, true)
end

IO.inspect(A.f(2), label: "A.f(2)")
