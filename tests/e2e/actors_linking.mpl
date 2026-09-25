# Linked actors: an abnormal exit ends the actor linked to it, a normal exit
# only removes the link.

actor crasher() do
  receive do
    n -> panic("crash #{n}")
  end
end

actor finisher() do
  receive do
    _ -> 0
  end
end

actor linked_to_crash() do
  let worker = spawn(crasher)
  link(worker)
  send(worker, 1)
  Timer.sleep(300)
  println("linked to a crash: still running")
end

actor linked_to_finish() do
  let worker = spawn(finisher)
  link(worker)
  send(worker, 1)
  Timer.sleep(300)
  println("linked to a normal exit: still running")
end

fn main() do
  spawn(linked_to_crash)
  spawn(linked_to_finish)
  Timer.sleep(600)
  println("link test done")
end
