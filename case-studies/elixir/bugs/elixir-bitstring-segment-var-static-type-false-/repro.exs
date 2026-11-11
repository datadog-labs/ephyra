defmodule A do
  def f(<<x::bits>>), do: <<x::binary>>
end

defmodule G do
  def f(x) when is_bitstring(x), do: <<x::binary>>
end

IO.inspect({A.f("ab"), G.f("ab")})
