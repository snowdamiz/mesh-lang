Application.load(:oban)
args = case System.argv() do
  ["--" | args] -> args
  args -> args
end

if args == ["session"] do
  Code.require_file("/proof/oban-session.ex")
end

if args in [["preflight"], ["run"], ["session"]] do
  [role] = args
  inetrc = System.get_env("ERL_INETRC")
  argv = "/proc/self/cmdline" |> File.read!() |> :binary.split(<<0>>, [:global, :trim_all])
  flag_argv =
    Map.new(["S", "SDcpu", "SDio", "A"], fn flag ->
      values = for [key, value] <- Enum.chunk_every(argv, 2, 1, :discard),
                   key in ["+" <> flag, "-" <> flag], do: value
      {"+" <> flag, values}
    end)

  addresses =
    if role == "preflight" do
      {:ok, {:hostent, _, _, :inet, 4, addresses}} = :inet.gethostbyname(~c"127.0.0.1", :inet)
      addresses
    else
      {:ok, address} = :inet.getaddr(~c"127.0.0.1", :inet)
      [address]
    end

  config = %{
    role: role,
    pid: System.pid() |> String.to_integer(),
    erl_flags: System.fetch_env!("ERL_FLAGS"),
    erl_inetrc: inetrc,
    inetrc_sha256: inetrc && Base.encode16(:crypto.hash(:sha256, File.read!(inetrc)), case: :lower),
    lookup: :inet_db.res_option(:lookup),
    loopback_addresses: Enum.map(addresses, &(List.to_string(:inet.ntoa(&1)))),
    flag_argv: flag_argv,
    schedulers: :erlang.system_info(:schedulers),
    schedulers_online: :erlang.system_info(:schedulers_online),
    dirty_cpu_schedulers: :erlang.system_info(:dirty_cpu_schedulers),
    dirty_cpu_schedulers_online: :erlang.system_info(:dirty_cpu_schedulers_online),
    dirty_io_schedulers: :erlang.system_info(:dirty_io_schedulers),
    async_threads: :erlang.system_info(:thread_pool_size)
  }

  device = if role == "session", do: :stderr, else: :stdio
  IO.puts(device, "BEAM_CONFIG " <> Jason.encode!(config))
end

unless args == ["preflight"], do: JobsParityOban.run(args)
