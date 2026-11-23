struct BenchRow do
  id :: Int
  a :: Int
  b :: Int
  c :: Int
  d :: Int
  e :: Int
  f :: Int
  g :: Int
end deriving(Row)

struct WorkerResult do
  finished_at :: Int
  checksums :: List<Int>
  calls_ns :: List<Int>
  call_errors :: List<String>
end

struct CallCadence do
  first_at_ns :: Int
  interval_ns :: Int
end

type Database do
  Postgres(pool :: PoolHandle)
  SQLite(conn :: SqliteConn)
  Idle
end

type TrialEvent do
  Start
  WorkerReady(worker :: Pid<Int>)
  WorkerDone(result :: WorkerResult)
  TickerReady
  TickerDone(lateness_ns :: List<Int>)
end

fn fail(message :: String) do
  println("error: " <> message)
  Process.exit(1)
  panic(message)
end

fn check(condition :: Bool, message :: String) do
  if !condition do
    fail(message)
  end
end

fn integer(text :: String) -> Int do
  case String.to_int(text) do
    Some(value) -> value
    None -> fail("expected an integer result")
  end
end

fn point_value(rows :: List<Map<String, String>>) -> Int do
  check(List.length(rows) == 1, "point query returned the wrong row count")
  integer(Map.get(List.head(rows), "a"))
end

fn row_score(rows :: List<Result<BenchRow, String>>) -> Int do
  check(List.length(rows) == 10000, "scan returned the wrong row count")
  let (_, score) = List.reduce(rows,
    (1, 0),
    fn((index, score), result) do
      let row = case result do
        Ok(row) -> row
        Err(message) -> fail(message)
      end
      check(row.id == index, "scan returned the wrong row order")
      let value = row.id
        + 2 * row.a
        + 3 * row.b
        + 4 * row.c
        + 5 * row.d
        + 6 * row.e
        + 7 * row.f
        + 8 * row.g
      let next = (index + 1, score + value)
      next
    end)
  score
end

fn pg_call(pool :: PoolHandle, mode :: String, key :: Int, write_id :: Int) -> Int!String do
  case mode do
    "pg_point" ->
      let rows = Pool.query(pool, "SELECT a FROM mesh_db_bench_rows WHERE id = $1", ["${key}"])?
      Ok(point_value(rows))
    "pg_typed" ->
      let rows = Pool.query_values(pool,
        "SELECT a FROM mesh_db_bench_rows WHERE id = $1",
        [Text("${key}")])?
      check(List.length(rows) == 1, "typed query returned the wrong row count")
      case Map.get(List.head(rows), "a") do
        Text(value) -> Ok(integer(value))
        _ -> Err("typed query returned the wrong value type")
      end
    "pg_write" ->
      Pool.execute(pool, "INSERT INTO mesh_db_bench_writes VALUES ($1)", ["${write_id}"])
    "pg_rows" ->
      let rows = Pool.query_as(pool,
        "SELECT id, a, b, c, d, e, f, g FROM mesh_db_bench_rows ORDER BY id",
        [],
        BenchRow.from_row)?
      Ok(row_score(rows))
    "pg_sleep" ->
      let rows = Pool.query(pool, "SELECT 1 AS a FROM pg_sleep(0.05)", [])?
      Ok(point_value(rows))
    _ -> Err("unknown PostgreSQL mode")
  end
end

fn sqlite_call(conn :: SqliteConn, mode :: String, key :: Int, write_id :: Int) -> Int!String do
  case mode do
    "sqlite_point" ->
      let rows = Sqlite.query(conn, "SELECT a FROM mesh_db_bench_rows WHERE id = ?", ["${key}"])?
      Ok(point_value(rows))
    "sqlite_write" ->
      let work = Env.get("DB_BENCH_SQLITE_WRITE_WORK", "")
      if work == "" do
        Sqlite.execute(conn, "INSERT INTO mesh_db_bench_writes VALUES (?)", ["${write_id}"])
      else
        Sqlite.execute(conn,
          "INSERT INTO mesh_db_bench_writes WITH RECURSIVE spin(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM spin WHERE n < CAST(? AS INTEGER)) SELECT ? FROM (SELECT sum(n) AS total FROM spin) WHERE total >= 0",
          [work, "${write_id}"])
      end
    "sqlite_rows" ->
      let rows = Sqlite.query(conn,
        "SELECT id, a, b, c, d, e, f, g FROM mesh_db_bench_rows ORDER BY id",
        [])?
      Ok(row_score(List.map(rows, BenchRow.from_row)))
    _ -> Err("unknown SQLite mode")
  end
end

fn db_call(db :: Database,
  mode :: String,
  worker :: Int,
  iteration :: Int,
  warmup :: Bool) -> Int!String do
  let key = (worker * 13 + iteration) % 10000 + 1
  let write_id = worker * 1000000 + iteration + 1
  let write_id = if warmup do
    -write_id
  else
    write_id
  end
  case db do
    Postgres(pool) -> pg_call(pool, mode, key, write_id)
    SQLite(conn) -> sqlite_call(conn, mode, key, write_id)
    Idle -> Err("idle trial called a database")
  end
end

fn expected_checksum(mode :: String, worker :: Int, iteration :: Int) -> Int do
  case mode do
    "pg_point" -> (worker * 13 + iteration) % 10000 + 2
    "pg_typed" -> (worker * 13 + iteration) % 10000 + 2
    "sqlite_point" -> (worker * 13 + iteration) % 10000 + 2
    "pg_rows" -> 1801860000
    "sqlite_rows" -> 1801860000
    _ -> 1
  end
end

actor bench_worker(coordinator :: Pid<TrialEvent>,
  db :: Database,
  mode :: String,
  worker :: Int,
  iterations :: Int,
  warmup :: Int) do
  let db = if mode == "sqlite_mixed" do
    case Sqlite.open(Env.get("DB_BENCH_SQLITE", "")) do
      Ok(conn) -> SQLite(conn)
      Err(message) -> fail(message)
    end
  else
    db
  end
  let query_mode = if mode == "sqlite_mixed" do
    "sqlite_point"
  else
    mode
  end
  for iteration in 0..warmup do
    let checksum = case db_call(db, query_mode, worker, iteration, true) do
      Ok(value) -> value
      Err(message) -> fail(message)
    end
    check(checksum == expected_checksum(query_mode, worker, iteration), "warmup checksum mismatch")
  end
  check(send(coordinator, WorkerReady(self())) == 0, "worker failed to announce readiness")
  let start_at = receive do
    timestamp -> timestamp
  end
  let spacing_ns = if mode == "pg_sleep" do
    Env.get_int("DB_BENCH_PG_SLEEP_SPACING_MS", 0) * 1000000
  else
    0
  end
  let cadence = CallCadence {
    first_at_ns: start_at + worker * spacing_ns,
    interval_ns: Env.get_int("DB_BENCH_ACTORS", 1) * spacing_ns
  }
  measure_calls(coordinator, db, query_mode, worker, cadence, 0, iterations, [], [], [])
  if mode == "sqlite_mixed" do
    case db do
      SQLite(conn) -> Sqlite.close(conn)
      _ -> fail("mixed reader did not open SQLite")
    end
  end
end

fn measure_calls(coordinator :: Pid<TrialEvent>,
  db :: Database,
  mode :: String,
  worker :: Int,
  cadence :: CallCadence,
  iteration :: Int,
  iterations :: Int,
  checksums :: List<Int>,
  calls_ns :: List<Int>,
  call_errors :: List<String>) do
  if cadence.interval_ns > 0 do
    let remaining = cadence.first_at_ns + iteration * cadence.interval_ns - Monotonic.now_nanos()
    if remaining > 0 do
      Timer.sleep((remaining + 999999) / 1000000)
    end
  end
  let started = Monotonic.now_nanos()
  let result = db_call(db, mode, worker, iteration, false)
  let finished = Monotonic.now_nanos()
  let (checksums, calls_ns, call_errors) = case result do
    Ok(checksum) ->
      check(checksum == expected_checksum(mode, worker, iteration), "timed call checksum mismatch")
      (List.append(checksums, checksum), List.append(calls_ns, finished - started), call_errors)
    Err(message) -> (checksums, calls_ns, List.append(call_errors, message))
  end
  if iteration + 1 == iterations do
    let result = WorkerResult {
      finished_at: finished,
      checksums: checksums,
      calls_ns: calls_ns,
      call_errors: call_errors
    }
    check(send(coordinator, WorkerDone(result)) == 0, "worker failed to report results")
  else
    measure_calls(coordinator,
      db,
      mode,
      worker,
      cadence,
      iteration + 1,
      iterations,
      checksums,
      calls_ns,
      call_errors)
  end
end

fn hold_mixed_writer(db :: Database) -> Int!String do
  case db do
    SQLite(conn) ->
      Sqlite.execute(conn, "BEGIN EXCLUSIVE", [])?
      Sqlite.execute(conn, "INSERT INTO mesh_db_bench_writes VALUES (0)", [])
    _ -> Err("mixed writer did not open SQLite")
  end
end

fn commit_mixed_writer(db :: Database) do
  case db do
    SQLite(conn) ->
      case Sqlite.commit(conn) do
        Ok(_) -> ()
        Err(message) -> fail(message)
      end
    _ -> fail("mixed writer did not open SQLite")
  end
end

actor ticker(coordinator :: Pid<TrialEvent>,
  chunks :: List<List<Int>>,
  samples :: List<Int>,
  first :: Bool) do
  if first do
    check(send(coordinator, TickerReady) == 0, "ticker failed to announce readiness")
  end
  let started = Monotonic.now_nanos()
  Timer.sleep(1)
  let delay = Monotonic.now_nanos() - started - 1000000
  let lateness = if delay > 0 do
    delay
  else
    0
  end
  let samples = List.append(samples, lateness)
  let stop = receive do
    _ -> true
    after 0 -> false
  end
  if stop do
    let all = List.flat_map(List.append(chunks, samples), fn(chunk) do chunk end)
    check(send(coordinator, TickerDone(all)) == 0, "ticker failed to report results")
  else if List.length(samples) == 128 do
    ticker(coordinator, List.append(chunks, samples), [], false)
  else
    ticker(coordinator, chunks, samples, false)
  end
end

actor trial(db :: Database, mode :: String, actors :: Int, iterations :: Int, warmup :: Int) do
  receive do
    Start -> ()
    _ -> fail("unexpected trial start message")
  end
  let coordinator = self()
  let workers = if mode == "idle" do
    0
  else
    actors
  end
  for worker in 0..workers do
    spawn(bench_worker, coordinator, db, mode, worker, iterations, warmup)
  end
  let callers = for _worker in 0..workers do
    receive do
      WorkerReady(worker) -> worker
      _ -> fail("unexpected worker readiness message")
    end
  end
  let gate = Env.get("DB_BENCH_GATE", "")
  case File.write(gate <> "/ready", "") do
    Ok(_) -> ()
    Err(message) -> fail(message)
  end
  while !File.exists(gate <> "/go") do
    Timer.sleep(1)
  end
  let clock = spawn(ticker, coordinator, [], [], true)
  receive do
    TickerReady -> ()
    _ -> fail("unexpected ticker readiness message")
  end
  if mode == "sqlite_mixed" do
    case hold_mixed_writer(db) do
      Ok(changes) -> check(changes == 1, "mixed writer did not insert its row")
      Err(message) -> fail(message)
    end
  end
  let started = Monotonic.now_nanos()
  if mode == "idle" do
    Timer.sleep(iterations)
  end
  for worker in callers do
    check(send(worker, started) == 0, "could not start benchmark worker")
  end
  let commit_started = if mode == "sqlite_mixed" do
    Timer.sleep(250)
    let before_commit = Monotonic.now_nanos()
    commit_mixed_writer(db)
    before_commit
  else
    0
  end
  let results = for _worker in 0..workers do
    receive do
      WorkerDone(result) -> result
      _ -> fail("unexpected worker result message")
    end
  end
  let finished = List.reduce(results,
    started,
    fn(latest, result :: WorkerResult) do
      if result.finished_at > latest do
        result.finished_at
      else
        latest
      end
    end)
  let elapsed_ns = if mode == "idle" do
    Monotonic.now_nanos() - started
  else
    finished - started
  end
  check(send(clock, 1) == 0, "could not stop ticker")
  let lateness_ns = receive do
    TickerDone(samples) -> samples
    _ -> fail("unexpected ticker result message")
  end
  let checksums = List.flat_map(results, fn(result :: WorkerResult) do result.checksums end)
  let calls_ns = List.flat_map(results, fn(result :: WorkerResult) do result.calls_ns end)
  let call_errors = List.flat_map(results, fn(result :: WorkerResult) do result.call_errors end)
  let readers_before_commit = List.length(List.filter(results,
    fn(result :: WorkerResult) do List.length(result.call_errors) == 0
      && result.finished_at < commit_started end))
  println(json {
    elapsed_ns: elapsed_ns,
    checksums: checksums,
    calls_ns: calls_ns,
    ticker_ns: lateness_ns,
    call_errors: call_errors,
    readers_before_commit: readers_before_commit
  })
end

fn open_db(mode :: String, pool_size :: Int) -> Database!String do
  if String.starts_with(mode, "pg_") do
    let pool = Pool.open(Env.get("DB_BENCH_URL", ""), pool_size, pool_size, 5000)?
    Ok(Postgres(pool))
  else if mode == "idle" do
    Ok(Idle)
  else
    let conn = Sqlite.open(Env.get("DB_BENCH_SQLITE", ""))?
    Ok(SQLite(conn))
  end
end

fn main() do
  let mode = Env.get("DB_BENCH_MODE", "")
  let actors = Env.get_int("DB_BENCH_ACTORS", 1)
  let iterations = Env.get_int("DB_BENCH_ITERATIONS", 1)
  let warmup = Env.get_int("DB_BENCH_WARMUP", 1)
  let pool_size = Env.get_int("DB_BENCH_POOL_SIZE", actors)
  let db = case open_db(mode, pool_size) do
    Ok(db) -> db
    Err(message) -> fail(message)
  end
  let coordinator = spawn(trial, db, mode, actors, iterations, warmup)
  check(Process.register("db_benchmark", coordinator) == 0, "could not register benchmark")
  check(send(coordinator, Start) == 0, "could not start benchmark")
  while Process.whereis("db_benchmark") == coordinator do
    Timer.sleep(1)
  end
  case db do
    Postgres(pool) -> Pool.close(pool)
    SQLite(conn) -> Sqlite.close(conn)
    Idle -> ()
  end
end
