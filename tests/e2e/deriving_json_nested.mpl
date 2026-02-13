struct Address do
  city :: String
  zip :: Int
end deriving(Json)

struct Person do
  name :: String
  addr :: Address
end deriving(Json)

fn show_person(p :: Person) do
  println(p.name)
  println(p.addr.city)
  println("${p.addr.zip}")
end

fn main() do
  let p = Person { name: "Bob", addr: Address { city: "NYC", zip: 10001 } }
  let json = Json.encode(p)
  println(json)

  let result = Person.from_json(json)
  case result do
    Ok(p2) -> show_person(p2)
    Err(e) -> println("Error: ${e}")
  end
end
