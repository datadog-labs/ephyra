defmodule A do
  def first(x) do
    case [x, 2] do
      [y | [_]] -> y
    end
  end
end

IO.inspect(A.first(:ok), label: "A.first(:ok) returned")
