from Probe import ready, finish, await_count, checkpoint, report

actor immediate(ticket :: Int) do
  finish(ticket, ticket)
end

actor parked(ticket :: Int) do
  ready(ticket)
  let value = receive do
    value -> value
  end
  finish(ticket, value)
end

fn start_immediate(first :: Int, end_id :: Int) -> Int do
  if first == end_id do
    0
  else
    let _ = spawn(immediate, first)
    start_immediate(first + 1, end_id)
  end
end

fn start_parked(first :: Int, end_id :: Int, sample :: Int) -> Int do
  if first == end_id do
    await_count(end_id, 0)
    if sample > 0 do
      checkpoint(sample)
    else
      0
    end
  else
    let child = spawn(parked, first)
    let _ = start_parked(first + 1, end_id, sample)
    send(child, first)
    0
  end
end

fn waves(first :: Int, total :: Int, width :: Int, hold :: Int, memory :: Int) -> Int do
  if first == total do
    0
  else
    let end_id = first + width
    let _ = if hold == 1 do
      let sample = if memory == 1 && first == 0 do
        1
      else if memory == 1 && end_id == total do
        3
      else
        0
      end
      start_parked(first, end_id, sample)
    else
      start_immediate(first, end_id)
    end
    let _ = await_count(end_id, 1)
    let _ = if memory == 1 && first == 0 do
      checkpoint(2)
    else
      0
    end
    waves(end_id, total, width, hold, memory)
  end
end

fn main() do
  let width = Env.get_int("PROBE_WIDTH", 1)
  let total = Env.get_int("PROBE_TOTAL", 16384)
  let hold = Env.get_int("PROBE_PARK", 1)
  let memory = Env.get_int("PROBE_MEMORY", 0)
  let _ = checkpoint(0)
  let _ = waves(0, total, width, hold, memory)
  let _ = if memory == 1 do checkpoint(4) else 0 end
  report(total, hold)
end
