defmodule PerformanceElixir.CPU do
  alias PerformanceElixir.Score

  def run(fixture_path, iterations, warmup) do
    records =
      fixture_path
      |> File.read!()
      |> JSON.decode!()
      |> fixture_records()

    size = tuple_size(records)

    if size == 0 and iterations + warmup > 0 do
      raise ArgumentError, "PERF_FIXTURES must contain records when iterations are nonzero"
    end

    visit(records, size, warmup, 0, 0)
    started = System.monotonic_time(:nanosecond)
    checksum = visit(records, size, iterations, 0, 0)
    elapsed = System.monotonic_time(:nanosecond) - started

    %{ops: iterations, elapsed_ns: elapsed, checksum: checksum}
  end

  defp fixture_records(records) when is_list(records) do
    records
    |> Enum.map(fn record ->
      case Score.record(record) do
        {:ok, record} -> record
        :error -> raise ArgumentError, "PERF_FIXTURES contains an invalid record"
      end
    end)
    |> List.to_tuple()
  end

  defp fixture_records(_records),
    do: raise(ArgumentError, "PERF_FIXTURES must contain a JSON array")

  defp visit(_records, _size, 0, _index, checksum), do: checksum

  defp visit(records, size, remaining, index, checksum) do
    score = records |> elem(index) |> Score.compute()
    next = if index + 1 == size, do: 0, else: index + 1
    visit(records, size, remaining - 1, next, checksum + score)
  end
end
