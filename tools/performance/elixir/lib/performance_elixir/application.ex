defmodule PerformanceElixir.Application do
  use Application

  alias PerformanceElixir.{Actor, CPU, HTTP, Recovery}

  @impl true
  def start(_type, _args) do
    child =
      case System.fetch_env!("PERF_MODE") do
        "actor" ->
          trial(fn ->
            Actor.run(integer_env("PERF_PAIRS", 1), iterations(), warmup())
          end)

        "cpu" ->
          trial(fn ->
            CPU.run(System.fetch_env!("PERF_FIXTURES"), iterations(), warmup())
          end)

        "recovery" ->
          trial(&Recovery.run/0)

        "serve" ->
          {Bandit,
           plug: HTTP,
           port: integer_env("PERF_PORT", 1, 65_535),
           http_1_options: [max_requests: 1],
           http_2_options: [enabled: false]}

        mode ->
          raise ArgumentError, "invalid PERF_MODE #{inspect(mode)}"
      end

    Supervisor.start_link([child], strategy: :one_for_one, max_restarts: 0)
  end

  defp trial(run) do
    Supervisor.child_spec(
      {Task,
       fn ->
         IO.puts(JSON.encode!(run.()))
         System.stop(0)
       end},
      restart: :transient
    )
  end

  defp iterations, do: integer_env("PERF_ITERS", 0)
  defp warmup, do: parse_integer(System.get_env("PERF_WARMUP", "0"), "PERF_WARMUP", 0, :infinity)

  defp integer_env(name, min, max \\ :infinity) do
    parse_integer(System.fetch_env!(name), name, min, max)
  end

  defp parse_integer(value, name, min, max) do
    case Integer.parse(value) do
      {number, ""} when number >= min and (max == :infinity or number <= max) ->
        number

      _ ->
        raise ArgumentError, "invalid #{name} #{inspect(value)}"
    end
  end
end
