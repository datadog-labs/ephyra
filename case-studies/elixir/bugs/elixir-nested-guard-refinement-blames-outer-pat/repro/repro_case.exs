defmodule ReproCase do
  def g(v) do
    case v do
      a = b ->
        _ = elem(a, 0)
        fn x when is_map(b) -> x end
    end
  end
end
IO.inspect(ReproCase.g({1}))
