struct Profile do
  name :: String
  bio :: Option<String>
  age :: Option<Int>
end deriving(Json)

fn show(r :: Result<Profile, String>) -> String do
  case r do
    Ok(p) -> Json.encode(p)
    Err(e) -> "err #{e}"
  end
end

fn main() do
  let with_bio = Profile { name: "Alice", bio: Some("Hello!"), age: Some(30) }
  let json1 = Json.encode(with_bio)
  println(json1)

  let without_bio = Profile { name: "Bob", bio: None, age: None }
  let json2 = Json.encode(without_bio)
  println(json2)

  println(show(Profile.from_json(json1)))
  println(show(Profile.from_json(json2)))
  println(show(Profile.from_json("{\"name\":\"Cy\",\"bio\":7,\"age\":null}")))
end
