fn value() -> Option < U64 > ! String do
  Ok(Some(("1"
    |> U64.parse()) ?))
end

fn main() do
  case value() do
    Ok( _) -> println("ok")
    Err( error) -> println(error)
  end
end
