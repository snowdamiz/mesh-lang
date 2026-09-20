# A job's result reaches the caller after the job actor, and its heap, are
# gone. Every result below is built at run time inside a job, the caller then
# allocates enough to collect several times, and only then reads the results.
#
# Build with --opt-level 2: at -O0 stale stack slots keep the values alive and
# hide the bug.

struct Report do
  title :: String
  lines :: List<String>
  score :: Int
end

fn churn(i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    let s = "garbage-${i}"
    churn(i + 1, n, acc + String.length(s))
  end
end

fn show_text(outcome :: Result<String, String>) -> Int do
  case outcome do
    Ok(text) -> println(text)
    Err(reason) -> println("failed: ${reason}")
  end
  0
end

fn show_lines(outcome :: Result<List<String>, String>) -> Int do
  case outcome do
    Ok(lines) -> println(String.join(lines, "+"))
    Err(reason) -> println("failed: ${reason}")
  end
  0
end

fn show_report(outcome :: Result<Report, String>) -> Int do
  case outcome do
    Ok(report) -> println("${report.title}: ${String.join(report.lines, "+")} = ${report.score}")
    Err(reason) -> println("failed: ${reason}")
  end
  0
end

fn show_maybe(outcome :: Result<Option<String>, String>) -> Int do
  case outcome do
    Ok(Some(text)) -> println("some ${text}")
    Ok(None) -> println("none")
    Err(reason) -> println("failed: ${reason}")
  end
  0
end

fn show_pair(outcome :: Result<(String, Int), String>) -> Int do
  case outcome do
    Ok(pair) -> do
      let (name, n) = pair
      println("${name} -> ${n}")
    end
    Err(reason) -> println("failed: ${reason}")
  end
  0
end

fn show_int(outcome :: Result<Int, String>) -> Int do
  case outcome do
    Ok(n) -> println("${n}")
    Err(reason) -> println("failed: ${reason}")
  end
  0
end

fn show_float(outcome :: Result<Float, String>) -> Int do
  case outcome do
    Ok(x) -> println("${x}")
    Err(reason) -> println("failed: ${reason}")
  end
  0
end

fn show_bool(outcome :: Result<Bool, String>) -> Int do
  case outcome do
    Ok(flag) -> println("${flag}")
    Err(reason) -> println("failed: ${reason}")
  end
  0
end

# A closure cannot be copied by type, so the job's heap is lent instead.
fn show_made(outcome :: Result<Fun(Int) -> String, String>) -> Int do
  case outcome do
    Ok(make) -> println(make(9))
    Err(reason) -> println("failed: ${reason}")
  end
  0
end

fn run_caller() -> Int do
  let seed = 41
  let prefix = "made-${seed}"
  let made = Job.await(Job.async(fn () -> fn (n :: Int) -> "${prefix}#${n}" end end))
  let text = Job.await(Job.async(fn () -> "payload-${seed + 1}" end))
  let lines = Job.await(Job.async(fn () -> ["line-${seed}", "line-${seed + 1}"] end))
  let report = Job.await(Job.async(fn () ->
    Report { title: "report-${seed}", lines: ["a-${seed}", "b-${seed}"], score: seed * 2 }
  end))
  let maybe = Job.await(Job.async(fn () -> Some("inner-${seed}") end))
  let pair = Job.await(Job.async(fn () -> ("pair-${seed}", seed + 2) end))
  let int = Job.await(Job.async(fn () -> seed * 3 end))
  let float = Job.await(Job.async(fn () -> 2.5 end))
  let bool = Job.await(Job.async(fn () -> seed > 40 end))
  let mapped = Job.map([1, 2, 3], fn (n) -> "mapped-${n * 100}" end)
  let piped = [4, 5] |> Job.map(fn (n) -> "piped-${n * 100}" end)
  let reports = Job.map([7, 8], fn (n) ->
    Report { title: "mapped-report-${n}", lines: ["m-${n}"], score: n }
  end)

  let total = churn(0, 200000, 0)

  show_text(text)
  show_lines(lines)
  show_report(report)
  show_maybe(maybe)
  show_pair(pair)
  show_int(int)
  show_float(float)
  show_bool(bool)
  show_text(List.get(mapped, 0))
  show_text(List.get(mapped, 2))
  show_text(List.get(piped, 1))
  show_report(List.get(reports, 1))
  show_made(made)
  total
end

actor caller() do
  receive do
    msg -> run_caller()
  end
end

fn main() do
  let pid = spawn(caller)
  send(pid, 1)
end
