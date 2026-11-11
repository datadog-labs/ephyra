defmodule A do
  def f(list), do: for(x <- items, into: (items = list; []), do: x)
end

IO.inspect(A.f([1, 2]), label: "A.f([1, 2])")
