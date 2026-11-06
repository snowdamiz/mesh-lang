import File

fn proof() -> Result<(), String> do
  let target = "$ROOT/state"
  let temporary = "$ROOT/state.tmp"
  File.write(target, "old")?
  File.write(temporary, "new")?
  File.sync(temporary)?
  File.rename(temporary, target)?
  File.sync("$ROOT")?
  println(File.read(target)?)
  println("#{File.exists(temporary)}")
  case File.rename(temporary, target) do
    Err(_) -> println("missing source")
    Ok(_) -> println("unexpected rename")
  end
  case File.sync(temporary) do
    Err(_) -> println("missing sync")
    Ok(_) -> println("unexpected sync")
  end
  println(File.read(target)?)
  File.delete(target)?
  Ok(nil)
end

fn main() do
  case proof() do
    Ok(_) -> nil
    Err(error) -> println(error)
  end
end
