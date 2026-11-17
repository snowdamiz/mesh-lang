struct Record do
  id :: Int
  values :: List<Int>
end

fn fail(message :: String) do
  IO.eprintln(message)
  Process.exit(1)
  # Process.exit is typed as Unit.
  panic(message)
end

fn check(condition :: Bool, message :: String) do
  if !condition do
    fail(message)
  end
end

fn decode_values(array :: Json,
  length :: Int,
  index :: Int,
  values :: List<Int>) -> List<Int>!String do
  if index == length do
    Ok(values)
  else
    let item = Json.array_get(array, index)?
    let value = Json.as_int(item)?
    decode_values(array, length, index + 1, List.append(values, value))
  end
end

fn decode_record(json_value :: Json) -> Record!String do
  let id_json = Json.object_get(json_value, "id")?
  let id = Json.as_int(id_json)?
  if id < 0 do
    Err("id must be nonnegative")
  else
    let array = Json.object_get(json_value, "values")?
    let length = Json.array_length(array)?
    let values = decode_values(array, length, 0, [])?
    Ok(Record { id: id, values: values })
  end
end

fn decode_records(array :: Json,
  length :: Int,
  index :: Int,
  records :: List<Record>) -> List<Record>!String do
  if index == length do
    Ok(records)
  else
    let value = Json.array_get(array, index)?
    let record = decode_record(value)?
    decode_records(array, length, index + 1, List.append(records, record))
  end
end

fn score_values(values :: List<Int>, length :: Int, index :: Int, total :: Int) -> Int do
  if index == length do
    total
  else
    score_values(values, length, index + 1, total + (index + 1) * List.get(values, index))
  end
end

fn score(record :: Record) -> Int do
  score_values(record.values, List.length(record.values), 0, record.id)
end

fn cpu_loop(records :: List<Record>,
  length :: Int,
  index :: Int,
  remaining :: Int,
  checksum :: Int) -> Int do
  if remaining == 0 do
    checksum
  else
    let total = checksum + score(List.get(records, index))
    let next = if index + 1 == length do
      0
    else
      index + 1
    end
    cpu_loop(records, length, next, remaining - 1, total)
  end
end

fn load_records() -> List<Record>!String do
  let contents = File.read(Env.get("PERF_FIXTURES", ""))?
  let array = Json.parse(contents)?
  let length = Json.array_length(array)?
  if length > 0 do
    decode_records(array, length, 0, [])
  else
    Err("PERF_FIXTURES must contain at least one record")
  end
end

fn run_cpu(iterations :: Int, warmup :: Int) do
  case load_records() do
    Ok(records) ->
      let length = List.length(records)
      let _ = cpu_loop(records, length, 0, warmup, 0)
      let started = Monotonic.now_nanos()
      let checksum = cpu_loop(records, length, 0, iterations, 0)
      let elapsed = Monotonic.now_nanos() - started
      println(json { ops: iterations, elapsed_ns: elapsed, checksum: checksum })
    Err(error) -> fail(error)
  end
end

fn health_handler(_request) do
  HTTP.response(200, json { ok: true })
end

fn score_body(body :: String) -> Record!String do
  let value = Json.parse(body)?
  decode_record(value)
end

fn score_handler(request) do
  case score_body(Request.body(request)) do
    Ok(record) -> HTTP.response(200, json { id: record.id, score: score(record) })
    Err(_) -> HTTP.response(400, json { error: "invalid record" })
  end
end

fn run_server() do
  let port = Env.get_int("PERF_PORT", 0)
  check(port > 0 && port <= 65535, "PERF_PORT must be a valid TCP port")
  let router = HTTP.router()
    |> HTTP.on_get("/health", health_handler)
    |> HTTP.on_post("/score", score_handler)
  Process.install_shutdown_signals()
  HTTP.serve(router, port)
  check(Process.shutdown_requested(), "HTTP server exited before shutdown")
end

type PairEvent do
  PairReady(caller :: Pid<Int>)
  PairDone(counter :: Int, finish :: Int, samples :: List<Int>)
end

actor counter_loop(caller :: Pid<Int>, total :: Int, remaining :: Int) do
  receive do
    amount ->
      let updated = total + amount
      check(send(caller, updated) == 0, "could not send counter reply")
      if remaining > 1 do
        counter_loop(caller, updated, remaining - 1)
      end
  end
end

actor pair_caller(worker_pid :: Option<Pid<Int>>,
  coordinator :: Pid<PairEvent>,
  index :: Int,
  iterations :: Int,
  expected :: Int,
  samples :: List<Int>,
  sampling :: Bool) do
  case worker_pid do
    None ->
      let caller = self()
      let worker = spawn(counter_loop, caller, 0, iterations + 1)
      check(send(worker, 0) == 0, "could not probe counter")
      receive do
        value -> check(value == 0, "counter must start at zero")
      end
      check(send(coordinator, PairReady(caller)) == 0, "could not send pair readiness")
      receive do
        command -> check(command == 1, "invalid pair start command")
      end
      pair_caller(Some(worker), coordinator, 1, iterations, 0, samples, sampling)
    Some(worker) ->
      let sampled = sampling && (index - 1) % 64 == 0
      let started = if sampled do
        Monotonic.now_nanos()
      else
        0
      end
      check(send(worker, index) == 0, "could not send counter request")
      let reply = receive do
        value -> value
      end
      let last = index == iterations
      let finish = if sampled || last do
        Monotonic.now_nanos()
      else
        0
      end
      let updated = expected + index
      check(reply == updated, "counter reply mismatch")
      let samples = if sampled do
        List.append(samples, finish - started)
      else
        samples
      end
      if last do
        check(send(coordinator, PairDone(reply, finish, samples)) == 0,
          "could not send pair result")
      else
        pair_caller(worker_pid, coordinator, index + 1, iterations, updated, samples, sampling)
      end
  end
end

actor actor_trial(pairs :: Int, iterations :: Int, warmup :: Int) do
  let warming = warmup > 0
  let count = if warming do
    warmup
  else
    iterations
  end
  let coordinator = self()
  for _index in 0..pairs do
    spawn(pair_caller, None, coordinator, 0, count, 0, [], !warming)
  end
  let callers = for _index in 0..pairs do
    receive do
      PairReady(caller) -> caller
      PairDone(_, _, _) -> fail("pair finished before start")
      after 5000 -> fail("pair did not become ready")
    end
  end
  let started = Monotonic.now_nanos()
  for caller in callers do
    check(send(caller, 1) == 0, "could not start pair")
  end
  let results = for _index in 0..pairs do
    receive do
      PairDone(counter, finish, samples) -> (counter, finish, samples)
      PairReady(_) -> fail("pair already started")
    end
  end
  if warming do
    actor_trial(pairs, iterations, 0)
  else
    let checksum = List.reduce(results, 0, fn total, (counter, _, _) -> total + counter end)
    # Each caller timestamps its final reply before reporting to the coordinator.
    let finish = List.reduce(results,
      started,
      fn latest, (_, finish, _) -> if finish > latest do
        finish
      else
        latest
      end end)
    let samples = List.flat_map(results, fn((_, _, samples)) -> samples end)
    println(json {
      ops: pairs * iterations,
      elapsed_ns: finish - started,
      checksum: checksum,
      rtt_ns: samples
    })
  end
end

type RecoveryRequest do
  AddCount(amount :: Int)
  ReadCount
  Crash
end

type RecoveryEvent do
  WorkerReady(recovering :: Bool, worker :: Pid<RecoveryRequest>)
  Count(value :: Int)
end

actor recovery_worker(recovering :: Bool, count :: Int, announced :: Bool) do
  let coordinator = Process.whereis("perf_recovery_coordinator")
  if !announced do
    check(send(coordinator, WorkerReady(recovering, self())) == 0,
      "could not send worker readiness")
  end
  receive do
    AddCount(amount) ->
      let updated = count + amount
      check(send(coordinator, Count(updated)) == 0, "could not send recovery counter")
      recovery_worker(recovering, updated, true)
    ReadCount ->
      check(send(coordinator, Count(count)) == 0, "could not send recovery state")
      recovery_worker(recovering, count, true)
    Crash -> panic("intentional benchmark worker failure")
  end
end

supervisor RecoverySupervisor do
  strategy: one_for_one

  child recovering do
    start: fn -> spawn(recovery_worker, true, 0, false) end
    restart: transient
    shutdown: 5000
  end

  child sibling do
    start: fn -> spawn(recovery_worker, false, 0, false) end
    restart: transient
    shutdown: 5000
  end
end

actor recovery_trial() do
  check(Process.register("perf_recovery_coordinator", self()) == 0,
    "could not register recovery coordinator")
  let _ = spawn(RecoverySupervisor)
  let children = for _index in 0..2 do
    receive do
      WorkerReady(recovering, worker) -> (recovering, worker)
      Count(_) -> fail("unexpected reply before worker startup")
      after 5000 -> fail("supervisor did not start both children")
    end
  end
  let (first_recovers, first) = List.get(children, 0)
  let (second_recovers, second) = List.get(children, 1)
  check(first_recovers != second_recovers, "expected two distinct recovery workers")
  let (failed, sibling) = if first_recovers do
    (first, second)
  else
    (second, first)
  end
  check(send(failed, AddCount(3)) == 0, "could not seed failed worker")
  receive do
    Count(value) -> check(value == 3, "failed worker seed mismatch")
    WorkerReady(_, _) -> fail("worker restarted before failure")
    after 5000 -> fail("failed worker did not respond before timing")
  end
  check(send(sibling, AddCount(7)) == 0, "could not seed sibling")
  receive do
    Count(value) -> check(value == 7, "sibling seed mismatch")
    WorkerReady(_, _) -> fail("worker restarted before failure")
    after 5000 -> fail("sibling did not respond before timing")
  end
  let started = Monotonic.now_nanos()
  check(send(failed, Crash) == 0, "could not send intentional failure")
  let replacement = receive do
    WorkerReady(true, worker) -> worker
    WorkerReady(false, _) -> fail("sibling restarted")
    Count(_) -> fail("unexpected reply during restart")
    after 5000 -> fail("supervisor did not restart the failed worker")
  end
  check(replacement != failed, "supervisor returned the original worker")
  check(send(replacement, ReadCount) == 0, "could not read replacement state")
  let empty = receive do
    Count(value) -> value
    WorkerReady(_, _) -> fail("unexpected restart after replacement")
    after 5000 -> fail("replacement did not respond")
  end
  let elapsed = Monotonic.now_nanos() - started
  check(empty == 0, "replacement must start with empty state")
  check(send(sibling, ReadCount) == 0, "could not read original sibling state")
  receive do
    Count(value) -> check(value == 7, "sibling state changed across restart")
    WorkerReady(_, _) -> fail("unexpected restart during sibling check")
    after 5000 -> fail("sibling did not respond after replacement")
  end
  println(json { ops: 1, elapsed_ns: elapsed, checksum: 1 })
end

fn main() do
  case Env.get("PERF_MODE", "") do
    "actor" ->
      let pairs = Env.get_int("PERF_PAIRS", 1)
      let iterations = Env.get_int("PERF_ITERS", 0)
      let warmup = Env.get_int("PERF_WARMUP", 0)
      check(pairs > 0 && iterations > 0 && warmup >= 0, "invalid actor trial size")
      let _ = spawn(actor_trial, pairs, iterations, warmup)
    "cpu" ->
      let iterations = Env.get_int("PERF_ITERS", 0)
      let warmup = Env.get_int("PERF_WARMUP", 0)
      check(iterations > 0 && warmup >= 0, "invalid CPU trial size")
      run_cpu(iterations, warmup)
    "serve" -> run_server()
    "recovery" ->
      let _ = spawn(recovery_trial)
    _ -> fail("PERF_MODE must be actor, cpu, serve, or recovery")
  end
end
