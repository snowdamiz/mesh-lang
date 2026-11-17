defmodule PerformanceElixir.Score do
  def record(%{"id" => id, "values" => values})
      when is_integer(id) and id >= 0 and is_list(values) do
    if Enum.all?(values, &is_integer/1), do: {:ok, {id, values}}, else: :error
  end

  def record(_record), do: :error

  def compute({id, values}), do: score_values(values, 1, id)

  defp score_values([], _weight, score), do: score

  defp score_values([value | rest], weight, score) do
    score_values(rest, weight + 1, score + weight * value)
  end
end
