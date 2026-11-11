defmodule A do
  def f(a = b) do
    _ = elem(a, 0)
    fn x when is_map(b) -> x end
  end
end

IO.inspect(is_function(A.f({1})), label: "is_function(A.f({1}))")
