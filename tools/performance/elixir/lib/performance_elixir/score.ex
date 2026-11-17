defmodule PerformanceElixir.Score do
  def record(%{"id" => id, "values" => values})
      when is_integer(id) and id in 0..1_000_000 and is_list(values) do
    if length(values) <= 256 and
         Enum.all?(values, &(is_integer(&1) and &1 in -1_000_000..1_000_000)) do
      {:ok, {id, values}}
    else
      :error
    end
  end

  def record(_record), do: :error

  def compute({id, values}), do: score_values(values, 1, id)

  defp score_values([], _weight, score), do: score

  defp score_values([value | rest], weight, score) do
    score_values(rest, weight + 1, score + weight * value)
  end
end
