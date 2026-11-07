##! Real generated-ABI effect regression. All URLs and database paths are supplied
##! by the owning loopback test. Native authority is outside these request bytes.

fn field(request :: Bytes, index :: Int) -> String!String do
  let parts = String.split(Bytes.to_utf8(request)?, "\n")
  if index >= List.length(parts) do Err("missing field") else Ok(List.get(parts, index)) end
end

fn hold(url :: String) -> Int!String do
  let request = Http.build(:post, url)
    |> Http.body("exact-attempt")
    |> Http.timeout(10000)
    |> Http.max_redirects(0)
  let response = Http.send(request)?
  Ok(response.status)
end

@export("authority_sql")
pub fn sql(request :: Bytes) -> Bytes!String do
  let db = Sqlite.open(field(request, 0)?)?
  let mode = field(request, 1)?
  let url = field(request, 2)?
  if mode == "commit" or mode == "raw-commit" or mode == "release" or mode == "accepted-commit" do
    if mode == "release" do
      Sqlite.execute(db, "SAVEPOINT owned", [])?
      0
    else
      Sqlite.begin(db)?
      0
    end
    Sqlite.execute(db, "INSERT INTO effects (body) VALUES (?)", ["staged"])?
    if mode != "accepted-commit" do
      Host.network_state(Bytes.from_utf8("commit-ready"))?
    end
    0
  else
    hold(url)?
    0
  end
  let result = if mode == "commit" or mode == "accepted-commit" do
    case Sqlite.commit(db) do Ok(_) -> Ok(1) Err(error) -> Err(error) end
  else
    if mode == "query-write" do
      case Sqlite.query(db, "INSERT INTO effects (body) VALUES ('returning') RETURNING body", []) do
        Ok(rows) -> Ok(List.length(rows))
        Err(error) -> Err(error)
      end
    else
      let statement = if mode == "raw-commit" do "COMMIT" else
        if mode == "release" do "RELEASE owned" else
          if mode == "schema" do "CREATE TABLE later (id INTEGER)" else
            "INSERT INTO effects (body) VALUES ('later')"
          end
        end
      end
      Sqlite.execute(db, statement, [])
    end
  end
  Sqlite.close(db)
  case result do Ok(_) -> Ok(Bytes.from_utf8("done")) Err(error) -> Err(error) end
end

@export("authority_http")
pub fn http(request :: Bytes) -> Bytes!String do
  hold(field(request, 0)?)?
  hold(field(request, 1)?)?
  Ok(Bytes.from_utf8("done"))
end

@export("authority_store")
pub fn store(request :: Bytes) -> Bytes!String do
  Host.network_state(Bytes.from_utf8("store-ready"))?
  Host.secure_store_put(request)?
  Host.secure_store_delete(request)
end

fn storage_context() -> Bytes!String do
  Bytes.from_hex("0111111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222233333333333333333333333333333333333333333333333333333333333333334444444444444444444444444444444444444444444444444444444444444444000e0000000000000001")
end

@export("authority_nonce")
pub fn nonce(request :: Bytes) -> Bytes!String do
  let key = case StorageKey.platform() do Ok(key) -> Ok(key) Err(_) -> Err("key") end ?
  Host.network_state(Bytes.from_utf8("nonce-ready"))?
  case StorageKey.seal_bytes(request, key, storage_context()?) do
    Ok(bytes) -> Ok(bytes)
    Err(_) -> Err("seal")
  end
end

actor delayed_store(value :: Bytes) do
  Timer.sleep(10)
  let _ = Host.network_state(Bytes.from_utf8("actor-ready"))
  let _ = Host.secure_store_put(value)
end

@export("authority_actor")
pub fn actor_start(request :: Bytes) -> Bytes!String do
  spawn(delayed_store, request)
  Ok(Bytes.from_utf8("spawned"))
end

@export("authority_error")
pub fn private_error(request :: Bytes) -> Bytes!String do
  Err("private-error-sentinel")
end

@export("authority_timer")
pub fn timer_start(request :: Bytes) -> Bytes!String do
  Timer.apply_after(10, fn() do
    let _ = Host.network_state(Bytes.from_utf8("timer-ready"))
    let _ = Host.secure_store_put(request)
  end)
  Ok(Bytes.from_utf8("scheduled"))
end

fn terminate_storage() -> Int!String do
  let bytes = Host.network_state(Bytes.from_utf8("terminate-ready"))?
  let db = Sqlite.open(Bytes.to_utf8(bytes)?)?
  Sqlite.begin(db)?
  Sqlite.execute(db, "INSERT INTO effects VALUES ('terminate')", [])?
  Ok(0)
end

actor terminating_store() do
  Timer.sleep(10)
terminate do
  let _ = terminate_storage()
end
end

@export("authority_terminate")
pub fn terminate_start(request :: Bytes) -> Bytes!String do
  spawn(terminating_store)
  Ok(Bytes.from_utf8("spawned"))
end

@export("authority_gc")
pub fn gc(request :: Bytes) -> Bytes!String do
  let lengths = for index in 0..1024 do
    let bytes = case Bytes.repeat(index % 251, 65536) do
      Ok(bytes) -> Ok(bytes)
      Err(_) -> Err("pressure")
    end ?
    Bytes.length(bytes)
  end
  if List.length(lengths) != 1024 do return Err("pressure") end
  Ok(request)
end

@export("authority_panic")
pub fn unwind(request :: Bytes) -> Bytes!String do
  panic("synthetic-panic")
  Ok(request)
end
