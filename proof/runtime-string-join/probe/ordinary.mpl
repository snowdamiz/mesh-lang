from Bindings import cpu_ns, checksum

fn measure(parts :: List<String>, extra :: List<String>, rounds :: Int, acc :: Int) -> Int do
  if rounds == 0 do
    return acc
  end
  let line = String.join(parts, " · ")
  let extended = String.join(extra, " · ")
  measure(parts, extra, rounds - 1, checksum(extended, checksum(line, acc)))
end

actor render() do
  let rows = ["label", "event=service.start", "region=eu-west-1", "user=Zoë", "path=/api/export", "bytes=1048576", "ok=true", "note=準備完了", "instance=blue-17"]
  let fields = List.drop(rows, 1)
  let extra = List.append(fields, "trace_id=6e46212af88443938749c0a8097cb922")
  let normal = String.join(fields, " · ")
  let extended = String.join(extra, " · ")
  println(Bytes.to_hex(Crypto.sha256(Bytes.from_utf8(normal))))
  println(Bytes.to_hex(Crypto.sha256(Bytes.from_utf8(extended))))
  let rounds = Env.get_int("PROBE_ROUNDS", 250000)
  let start = cpu_ns()
  let sum = measure(fields, extra, rounds, 0)
  let elapsed = cpu_ns() - start
  println("#{sum} #{elapsed}")
end

fn main() do
  let _ = spawn(render)
end
