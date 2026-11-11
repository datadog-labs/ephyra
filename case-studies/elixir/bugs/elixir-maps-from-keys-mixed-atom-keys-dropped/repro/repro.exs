defmodule A do
  def f do
    case Map.from_keys([:a, "b"], 1) do
      %{a: value} -> value
      _ -> :no_key_a
    end
  end
end

IO.inspect(A.f(), label: "A.f()")
