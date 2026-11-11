defmodule A do
  def put(x) do
    Map.put(x, :key, (case x do
      [_ | _] -> throw(:list_clause_ran)
      _ -> :other
    end))
  end
end

IO.inspect(try(do: A.put([1]), catch: (value -> {:thrown, value})))
