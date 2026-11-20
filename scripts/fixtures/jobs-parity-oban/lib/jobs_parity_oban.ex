defmodule JobsParityOban.Repo do
  use Ecto.Repo, otp_app: :jobs_parity_oban, adapter: Ecto.Adapters.Postgres
end

defmodule JobsParityOban.Migration do
  use Ecto.Migration

  def up do
    Oban.Migration.up(prefix: prefix(), create_schema: false, unlogged: false)
  end
end

defmodule JobsParityOban do
  alias JobsParityOban.{Migration, Repo, Worker}

  @batch_size 128
  @rescue_after_ms 5_000
  @rescue_interval_ms 250

  def run(["versions"]) do
    {:ok, spec} = :application.get_key(:oban, :vsn)
    otp_release = System.otp_release()

    otp_version =
      Path.join([List.to_string(:code.root_dir()), "releases", otp_release, "OTP_VERSION"])

    erlang =
      if File.regular?(otp_version),
        do: otp_version |> File.read!() |> String.trim(),
        else: otp_release

    print_json(%{oban: List.to_string(spec), erlang: erlang, elixir: System.version()})
  end

  def run([action]) when action in ["migrate", "enqueue", "run"] do
    {:ok, _} = Application.ensure_all_started(:jobs_parity_oban)

    namespace = System.fetch_env!("JOBS_PARITY_NAMESPACE")
    validate_namespace!(namespace)
    slots = if action == "run", do: env_integer!("JOBS_PARITY_SLOTS", 1)
    if slots, do: Process.flag(:trap_exit, true)
    pool_size = if slots, do: slots + 2, else: 2
    runner = if slots, do: System.fetch_env!("JOBS_PARITY_RUNNER"), else: action

    {:ok, repo} =
      Repo.start_link(
        connection_options(System.fetch_env!("JOBS_PARITY_DB_URL")) ++
          [
            pool_size: pool_size,
            log: false,
            parameters: [application_name: "jobs_parity_oban:#{namespace}:#{runner}"]
          ]
      )

    try do
      case action do
        "migrate" -> migrate(namespace)
        "enqueue" -> enqueue(namespace, runner)
        "run" -> run_producer(namespace, runner, slots)
      end
    after
      Supervisor.stop(repo)
    end
  end

  def run(_args), do: raise(ArgumentError, "expected one of: migrate, enqueue, run, versions")

  defp migrate(namespace) do
    Repo.query!(~s(CREATE SCHEMA IF NOT EXISTS "#{namespace}"))

    Ecto.Migrator.up(Repo, 1, Migration,
      prefix: namespace,
      log: false,
      log_migrator_sql: false
    )
  end

  defp enqueue(namespace, runner) do
    path = System.fetch_env!("JOBS_PARITY_INPUT")
    true = Path.type(path) == :absolute
    %{"jobs" => jobs, "rollback" => rollback} = input = path |> File.read!() |> Jason.decode!()
    true = is_list(jobs) and is_boolean(rollback)
    request_id = Map.get(input, "request_id", "batch")
    true = is_binary(request_id)

    hold_ms = env_integer!("JOBS_PARITY_HOLD_MS", 0, "0")
    changesets = Enum.map(jobs, &new_job(&1, hold_ms))

    {:ok, oban} =
      Oban.start_link(oban_options(namespace, runner) ++ [stager: false, peer: false])

    try do
      result =
        Repo.transaction(fn ->
          %{rows: [[xid]]} = Repo.query!("SELECT pg_current_xact_id()::text")

          Repo.query!(
            """
            INSERT INTO public.mesh_jobs_parity_product (engine, namespace, request_id)
            VALUES ('oban', $1, $2)
            ON CONFLICT DO NOTHING
            """,
            [namespace, request_id]
          )

          count =
            changesets
            |> Enum.chunk_every(@batch_size)
            |> Enum.reduce(0, fn batch, count -> count + length(Oban.insert_all(batch)) end)

          receipt = %{count: count, xid: xid}
          if rollback, do: Repo.rollback(receipt), else: receipt
        end)

      case result do
        {:ok, receipt} -> print_json(receipt)
        {:error, receipt} -> print_json(receipt)
      end
    after
      Supervisor.stop(oban)
    end
  end

  defp new_job(
         %{"key" => key, "payload" => payload, "fail_first" => fail_first} = job,
         legacy_hold_ms
       )
       when is_binary(key) and is_binary(payload) and is_boolean(fail_first) do
    hold_ms = Map.get(job, "hold_ms", legacy_hold_ms)
    true = is_integer(hold_ms) and hold_ms >= 0

    Worker.new(%{
      "key" => key,
      "payload" => payload,
      "hold_ms" => hold_ms,
      "fail_first" => fail_first
    })
  end

  defp run_producer(namespace, runner, slots) do
    poll_ms = env_integer!("JOBS_PARITY_POLL_MS", 1)

    lifeline =
      case System.fetch_env!("JOBS_PARITY_MODE") do
        mode when mode in ["dense", "sparse"] -> false
        "fault" -> [interval: @rescue_interval_ms, rescue_after: @rescue_after_ms]
        _other -> raise ArgumentError, "JOBS_PARITY_MODE must be dense, sparse, or fault"
      end

    # Each VM polls independently because the notifier only relays inside that VM.
    options =
      oban_options(namespace, runner) ++
        [
          queues: [parity: slots],
          peer: Oban.Peers.Isolated,
          stager: [interval: poll_ms],
          lifeline: lifeline,
          shutdown_grace_period: 15_000
        ]

    parent = self()

    on_term = fn ->
      send(parent, :shutdown)
      :ok
    end

    {:ok, __MODULE__} = System.trap_signal(:sigterm, __MODULE__, on_term)
    spawn(fn ->
      if IO.read(:stdio, :line) == "stop\n", do: send(parent, :shutdown)
    end)

    Repo.query!(~s(SELECT 1 FROM "#{namespace}".oban_jobs LIMIT 0))
    {:ok, oban} = Oban.start_link(options)
    repo = Process.whereis(Repo)

    try do
      %{limit: ^slots} = Oban.check_queue(queue: :parity)
      IO.puts("READY")

      receive do
        :shutdown -> :ok
        {:EXIT, pid, reason} when pid == oban or pid == repo -> exit(reason)
      end
    after
      Supervisor.stop(oban)
    end
  end

  defp oban_options(namespace, runner) do
    [
      repo: Repo,
      prefix: namespace,
      node: runner,
      engine: Oban.Engines.Basic,
      notifier: Oban.Notifiers.Isolated,
      insert_trigger: false,
      dispatch_cooldown: 5,
      log: false
    ]
  end

  defp connection_options(url) do
    uri = URI.parse(url)
    query = URI.decode_query(uri.query || "")
    {sslmode, query} = Map.pop(query, "sslmode")
    {root_cert, query} = Map.pop(query, "sslrootcert")

    ssl_opts =
      case sslmode do
        nil -> []
        "disable" -> [ssl: false]
        "verify-full" when is_binary(root_cert) -> [ssl: [cacertfile: root_cert]]
        "verify-full" -> [ssl: true]
        _other -> raise ArgumentError, "use sslmode=disable or sslmode=verify-full"
      end

    query = if sslmode, do: Map.delete(query, "ssl"), else: query
    [url: URI.to_string(%{uri | query: URI.encode_query(query)})] ++ ssl_opts
  end

  defp validate_namespace!(namespace) do
    if not Regex.match?(~r/\A[a-z][a-z0-9_]{0,62}\z/, namespace) or
         String.starts_with?(namespace, "pg_") or namespace in ["public", "information_schema"] do
      raise ArgumentError,
            "JOBS_PARITY_NAMESPACE must be a dedicated lower snake case PostgreSQL schema"
    end
  end

  defp env_integer!(name, minimum, default \\ nil) do
    case Integer.parse(System.get_env(name) || default || System.fetch_env!(name)) do
      {number, ""} when number >= minimum -> number
      _invalid -> raise ArgumentError, "#{name} must be an integer of at least #{minimum}"
    end
  end

  defp print_json(value), do: IO.puts(Jason.encode!(value))
end
