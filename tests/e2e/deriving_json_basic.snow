struct User do
  name :: String
  age :: Int
  score :: Float
  active :: Bool
end deriving(Json)

fn main() do
  let u = User { name: "Alice", age: 30, score: 95.5, active: true }
  let json = Json.encode(u)
  println(json)

  let result = User.from_json(json)
  case result do
    Ok(u2) -> println("${u2.name} ${u2.age} ${u2.active}")
    Err(e) -> println("Error: ${e}")
  end
end
