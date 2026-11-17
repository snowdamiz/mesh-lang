defmodule PerformanceElixir.Actor do
  def run(pairs, iterations, warmup) do
    coordinator = self()

    callers =
      for _ <- 1..pairs do
        spawn_link(fn -> caller(coordinator, iterations, warmup) end)
      end

    Enum.each(callers, fn _ ->
      receive do
        :ready -> :ok
      end
    end)

    start = System.monotonic_time(:nanosecond)
    Enum.each(callers, &send(&1, :run))

    {checksum, finish, samples} =
      Enum.reduce(callers, {0, start, []}, fn _, {checksum, finish, samples} ->
        receive do
          {:done, counter, finished_at, rtts} ->
            {checksum + counter, max(finish, finished_at), [rtts | samples]}
        end
      end)

    %{
      ops: pairs * iterations,
      elapsed_ns: finish - start,
      checksum: checksum,
      rtt_ns: List.flatten(samples)
    }
  end

  defp caller(coordinator, iterations, warmup) do
    caller = self()
    worker = spawn_link(fn -> counter(caller, 0) end)

    exchange(worker, 1, warmup, 0, [], false)
    send(worker, :reset)

    receive do
      :reset_done -> send(coordinator, :ready)
    end

    receive do
      :run ->
        {counter, samples} = exchange(worker, 1, iterations, 0, [], true)
        finish = System.monotonic_time(:nanosecond)
        send(coordinator, {:done, counter, finish, Enum.reverse(samples)})
        send(worker, :stop)
    end
  end

  defp exchange(_worker, iteration, limit, counter, samples, _sample?) when iteration > limit do
    {counter, samples}
  end

  defp exchange(worker, iteration, limit, counter, samples, sample?) do
    started = if sample? and rem(iteration - 1, 64) == 0, do: System.monotonic_time(:nanosecond)
    send(worker, iteration)

    reply =
      receive do
        {:counter, value} -> value
      end

    samples =
      if started do
        [System.monotonic_time(:nanosecond) - started | samples]
      else
        samples
      end

    expected = counter + iteration

    if reply != expected do
      raise "actor reply mismatch at iteration #{iteration}: expected #{expected}, got #{reply}"
    end

    exchange(worker, iteration + 1, limit, reply, samples, sample?)
  end

  defp counter(caller, value) do
    receive do
      :reset ->
        send(caller, :reset_done)
        counter(caller, 0)

      :stop ->
        :ok

      increment when is_integer(increment) ->
        value = value + increment
        send(caller, {:counter, value})
        counter(caller, value)
    end
  end
end
