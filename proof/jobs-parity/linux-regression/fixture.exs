Application.load(:oban)
args = case System.argv() do
  ["--" | args] -> args
  args -> args
end

if args == ["session"] do
  Code.require_file("/proof/oban-session.ex")
end

if args == ["run"] do
  config = %{
    erl_flags: System.fetch_env!("ERL_FLAGS"),
    argv: "/proc/self/cmdline" |> File.read!() |> :binary.split(<<0>>, [:global, :trim_all]),
    schedulers: :erlang.system_info(:schedulers),
    schedulers_online: :erlang.system_info(:schedulers_online),
    dirty_cpu_schedulers: :erlang.system_info(:dirty_cpu_schedulers),
    dirty_cpu_schedulers_online: :erlang.system_info(:dirty_cpu_schedulers_online),
    dirty_io_schedulers: :erlang.system_info(:dirty_io_schedulers),
    async_threads: :erlang.system_info(:thread_pool_size)
  }

  IO.puts("BEAM_CONFIG " <> Jason.encode!(config))
end

JobsParityOban.run(args)
