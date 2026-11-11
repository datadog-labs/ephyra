defmodule A do
  def f(<<x::bitstring>>) when is_binary(x), do: :binary
  def f(_), do: :other

  def g, do: f(<<1::1>>) == :other
end

IO.inspect(A.f(<<1::1>>), label: "A.f(<<1::1>>)")
