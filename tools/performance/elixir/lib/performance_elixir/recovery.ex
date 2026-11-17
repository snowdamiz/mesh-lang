defmodule PerformanceElixir.Recovery do
  alias PerformanceElixir.Recovery.Worker

  def run do
    observer = self()

    children =
      for id <- [:failed, :sibling] do
        Supervisor.child_spec({Worker, {id, observer}}, id: id)
      end

    {:ok, supervisor} = Supervisor.start_link(children, strategy: :one_for_one)
    failed = ready_worker(:failed)
    sibling = ready_worker(:sibling)

    1 = GenServer.call(failed, :increment)
    1 = GenServer.call(sibling, :increment)

    started = System.monotonic_time(:nanosecond)
    GenServer.cast(failed, :fail)
    replacement = ready_worker(:failed)
    replacement_state = GenServer.call(replacement, :state)
    elapsed = System.monotonic_time(:nanosecond) - started

    true = replacement != failed
    0 = replacement_state
    1 = GenServer.call(sibling, :state)
    :ok = Supervisor.stop(supervisor)

    %{ops: 1, elapsed_ns: elapsed, checksum: 1}
  end

  defp ready_worker(id) do
    receive do
      {:ready, ^id, pid} -> pid
    end
  end
end

defmodule PerformanceElixir.Recovery.Worker do
  use GenServer

  def start_link(args), do: GenServer.start_link(__MODULE__, args)

  @impl true
  def init({id, observer}) do
    send(observer, {:ready, id, self()})
    {:ok, 0}
  end

  @impl true
  def handle_call(:increment, _from, state), do: {:reply, state + 1, state + 1}
  def handle_call(:state, _from, state), do: {:reply, state, state}

  @impl true
  def handle_cast(:fail, state), do: {:stop, {:shutdown, :benchmark_failure}, state}
end
