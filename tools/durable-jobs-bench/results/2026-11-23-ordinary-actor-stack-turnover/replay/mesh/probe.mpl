@native("actor_probe_ready")
pub fn ready(ticket :: Int) -> Int
@native("actor_probe_finish")
pub fn finish(ticket :: Int, received :: Int) -> Int
@native("actor_probe_await")
pub fn await_count(total :: Int, finished :: Int) -> Int
@native("actor_probe_checkpoint")
pub fn checkpoint(phase :: Int) -> Int
@native("actor_probe_report")
pub fn report(total :: Int, held :: Int) -> Int
