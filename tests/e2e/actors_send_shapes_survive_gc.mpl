# Every common message shape must arrive as the receiver's own copy: the sender
# drops each value and keeps collecting while the receivers are still asleep.
# Strings are built at run time, the same size as the churned garbage, so a
# freed copy would be overwritten. Build with --opt-level 2.
struct Job do
  name :: String
  tries :: Int
end

# One word wide, so a tuple field or argument slot holds it without a box.
struct Batch do
  names :: List<String>
end

type Note do
  Named(String)
  Pair(String, Int)
  Wrapped(Job)
  Blank
end

fn say(label :: String, value :: String) -> Int do
  Timer.sleep(500)
  println("${label}=${value}")
  0
end

fn show_note(note :: Note) -> Int do
  case note do
    Named(name) -> say("named", name)
    Pair(name, n) -> say("pair", "${name}/${n}")
    Wrapped(job) -> say("wrapped", "${job.name}/${job.tries}")
    Blank -> say("blank", "-")
  end
end

fn show_tuple(pair :: (String, Int)) -> Int do
  let (name, n) = pair
  say("tuple", "${name}/${n}")
end

fn show_pairs(pairs :: List<(String, Int)>) -> Int do
  let (name, n) = List.get(pairs, 1)
  say("pairs", "${name}/${n}")
end

fn show_batch(pair :: (Batch, Int)) -> Int do
  let (batch, n) = pair
  say("batch", "${String.join(batch.names, "+")}/${n}")
end

fn show_job(job :: Job) -> Int do
  say("struct", "${job.name}/${job.tries}")
end

fn show_list(items :: List<String>) -> Int do
  say("list", "${List.get(items, 0)}+${List.get(items, 1)}")
end

fn show_jobs(jobs :: List<Job>) -> Int do
  let last = List.get(jobs, 1)
  say("jobs", "${last.name}/${last.tries}")
end

fn show_map(index :: Map<String, String>) -> Int do
  say("map", Map.get(index, "key-1000001"))
end

fn show_option(found :: Option<String>) -> Int do
  case found do
    Some(name) -> say("option", name)
    None -> say("option", "none")
  end
end

fn show_result(outcome :: Result<Int, String>) -> Int do
  case outcome do
    Ok(n) -> say("result", "${n}")
    Err(reason) -> say("result", reason)
  end
end

actor r_named() do
  receive do
    m -> show_note(m)
  end
end

actor r_pair() do
  receive do
    m -> show_note(m)
  end
end

actor r_wrapped() do
  receive do
    m -> show_note(m)
  end
end

actor r_tuple() do
  receive do
    m -> show_tuple(m)
  end
end

actor r_pairs() do
  receive do
    m -> show_pairs(m)
  end
end

actor r_batch() do
  receive do
    m -> show_batch(m)
  end
end

actor r_job() do
  receive do
    m -> show_job(m)
  end
end

actor r_list() do
  receive do
    m -> show_list(m)
  end
end

actor r_jobs() do
  receive do
    m -> show_jobs(m)
  end
end

actor r_map() do
  receive do
    m -> show_map(m)
  end
end

actor r_option() do
  receive do
    m -> show_option(m)
  end
end

actor r_result() do
  receive do
    m -> show_result(m)
  end
end

fn churn(i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    let s = "garbage-${i}"
    churn(i + 1, n, acc + String.length(s))
  end
end

fn run_sender() -> Int do
  send(spawn(r_named), Named("named-${1000 + 1}"))
  send(spawn(r_pair), Pair("paired-${100 + 2}", 7))
  send(spawn(r_wrapped), Wrapped(Job { name: "boxjob-${100 + 3}", tries: 3 }))
  send(spawn(r_tuple), ("tupled-${100 + 4}", 4))
  send(spawn(r_pairs), [("pairs-a-${100 + 13}", 13), ("pairs-b-${100 + 14}", 14)])
  send(spawn(r_batch), (Batch { names: ["batch-a-${100 + 15}", "batch-b-${100 + 16}"] }, 15))
  send(spawn(r_job), Job { name: "struct-${100 + 5}", tries: 5 })
  send(spawn(r_list), ["listed-${100 + 6}", "listed-${100 + 7}"])
  send(spawn(r_jobs), [Job { name: "jobs-a-${100 + 8}", tries: 8 }, Job { name: "jobs-b-${100 + 9}", tries: 9 }])
  send(spawn(r_map), Map.put(Map.new(), "key-${1000000 + 1}", "mapped-${100 + 10}"))
  send(spawn(r_option), Some("option-${100 + 11}"))
  send(spawn(r_result), Err("result-${100 + 12}"))
  churn(0, 200000, 0)
end

actor sender() do
  receive do
    msg -> run_sender()
  end
end

fn main() do
  let pid = spawn(sender)
  send(pid, 1)
end
