defmodule A do
  def f(<<x::bits>>), do: <<x::binary>>
end

IO.inspect(A.f("ab"), label: "A.f(\"ab\")")
