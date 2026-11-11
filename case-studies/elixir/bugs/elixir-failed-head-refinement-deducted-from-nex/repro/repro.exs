defmodule A do
  def f(x = y) when is_integer(x) and is_atom(y), do: raise("unreachable")
  def f(_), do: :other
end

defmodule B do
  def g, do: A.f(:a)
end

IO.inspect(B.g(), label: "B.g() returned")
