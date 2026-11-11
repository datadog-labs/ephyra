defmodule Repro do
  def f(a = b) do
    _ = elem(a, 0)
    fn x when is_map(b) -> x end
  end
end

IO.inspect(Repro.f({1}), label: "f({1}) returned")
