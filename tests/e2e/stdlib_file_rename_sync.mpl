import File

fn save_proof() -> Bool!String do
  let temp = "/tmp/mesh_test_rename_sync.tmp"
  let target = "/tmp/mesh_test_rename_sync.txt"
  File.write(target, "old")?
  File.write(temp, "new")?
  File.sync(temp)?
  File.rename(temp, target)?
  println(File.read(target)?)
  println("#{File.exists(temp)}")
  case File.rename(temp, target) do
    Err(_) -> println("missing source")
    Ok(_) -> println("renamed nothing")
  end
  case File.sync(temp) do
    Err(_) -> println("missing sync")
    Ok(_) -> println("synced nothing")
  end
  File.delete(target)?
  Ok(true)
end

fn main() do
  case save_proof() do
    Err(error) -> println(error)
    Ok(_) -> nil
  end
end
