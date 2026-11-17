defmodule PerformanceElixir.Score do
  def record(%{"id" => id, "values" => values}) when is_list(values) do
    with {:ok, id} when id >= 0 <- integral(id),
         true <- length(values) <= 256,
         {:ok, values} <- normalize(values) do
      {:ok, {id, values}}
    else
      _ -> :error
    end
  end

  def record(_record), do: :error

  defp integral(value) when is_integer(value) and value in -1_000_000..1_000_000,
    do: {:ok, value}

  defp integral(value) when is_float(value) and value >= -1_000_000 and value <= 1_000_000 do
    integer = trunc(value)
    if value == integer, do: {:ok, integer}, else: :error
  end

  defp integral(_value), do: :error

  defp normalize(values) do
    if Enum.all?(values, &(is_integer(&1) and &1 in -1_000_000..1_000_000)) do
      {:ok, values}
    else
      normalize(values, [])
    end
  end

  defp normalize([], acc), do: {:ok, Enum.reverse(acc)}

  defp normalize([value | rest], acc) do
    case integral(value) do
      {:ok, integer} -> normalize(rest, [integer | acc])
      :error -> :error
    end
  end

  def compute({id, values}), do: score_values(values, 1, id)

  defp score_values([], _weight, score), do: score

  defp score_values([value | rest], weight, score) do
    score_values(rest, weight + 1, score + weight * value)
  end
end
