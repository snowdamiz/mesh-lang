fn field(request :: Bytes, index :: Int) -> String!String do
  let parts = String.split(Bytes.to_utf8(request)?, "\n")
  if index >= List.length(parts) do Err("missing field") else Ok(List.get(parts, index)) end
end

fn attempt(url :: String) -> Int!String do
  let response = Http.send(Http.build(:post, url) |> Http.body("original") |> Http.timeout(15000))?
  Ok(response.status)
end

@export("progress_old")
pub fn old(request :: Bytes) -> Bytes!String do
  let db = Sqlite.open(field(request, 0)?)?
  attempt(field(request, 1)?)?
  Sqlite.execute(db, "INSERT INTO effects VALUES ('old')", [])?
  Host.secure_store_put(Bytes.from_utf8("old-write"))?
  attempt(field(request, 2)?)?
  Sqlite.close(db)
  Ok(Bytes.from_utf8("private-success"))
end

@export("progress_error")
pub fn old_error(request :: Bytes) -> Bytes!String do
  attempt(field(request, 1)?)?
  Err("private-error")
end

@export("progress_load")
pub fn load(request :: Bytes) -> Bytes!String do
  let db = Sqlite.open(Bytes.to_utf8(request)?)?
  let rows = Sqlite.query(db, "SELECT body FROM effects", [])?
  Sqlite.close(db)
  if List.length(rows) != 0 do return Err("unexpected write") end
  Ok(Bytes.from_utf8("loaded"))
end

@export("progress_status")
pub fn status(request :: Bytes) -> Bytes!String do
  Ok(request)
end
