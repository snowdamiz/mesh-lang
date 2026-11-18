defmodule PerformanceElixir.Actor do
  def run(pairs, iterations, warmup) do
    if warmup > 0, do: phase(pairs, warmup, false)
    phase(pairs, iterations, true)
  end

  defp phase(pairs, iterations, measure) do
    coordinator = self()

    callers =
      for _ <- 1..pairs do
        spawn_link(fn -> caller(coordinator, iterations, measure) end)
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

  defp caller(coordinator, iterations, measure) do
    caller = self()

    worker =
      spawn_link(fn ->
        send(caller, :worker_ready)
        counter(caller, 0)
      end)

    receive do
      :worker_ready -> send(coordinator, :ready)
    end

    receive do
      :run ->
        {counter, samples, final_reply_at} = exchange(worker, 1, iterations, 0, [], measure)
        send(coordinator, {:done, counter, final_reply_at, Enum.reverse(samples)})
        send(worker, :stop)
    end
  end

  defp exchange(worker, iteration, limit, counter, samples, measure) do
    offset = rem(iteration - 1, 64)
    phase = rem(div(iteration - 1, 64) * 17, 64)
    final = iteration == limit
    sampled = measure and (offset == phase or (final and offset < phase))
    started = if sampled, do: System.monotonic_time(:nanosecond)
    send(worker, iteration)

    reply =
      receive do
        {:counter, value} -> value
      end

    received = if started || final, do: System.monotonic_time(:nanosecond)
    samples = if started, do: [received - started | samples], else: samples

    expected = counter + iteration

    if reply != expected do
      raise "actor reply mismatch at iteration #{iteration}: expected #{expected}, got #{reply}"
    end

    if final do
      {reply, samples, received}
    else
      exchange(worker, iteration + 1, limit, reply, samples, measure)
    end
  end

  defp counter(caller, value) do
    receive do
      :stop ->
        :ok

      increment when is_integer(increment) ->
        value = value + increment
        send(caller, {:counter, value})
        counter(caller, value)
    end
  end
end
