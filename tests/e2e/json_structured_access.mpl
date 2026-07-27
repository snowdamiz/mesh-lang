fn print_bool(value :: Bool) do
  println("#{value}")
end

fn print_int(value :: Int) do
  println("#{value}")
end

fn print_float(value :: Float) do
  println("#{value}")
end

fn inspect() -> Int ! String do
  let root = Json.parse(
    """{"values":["first","second"],"active":true,"count":7,"ratio":1.5,"empty":null}"""
  ) ?
  let values = (root
    |> Json.object_get("values")) ?
  (values
    |> Json.array_length()) ?
    |> print_int()
  ((values
    |> Json.array_get(1)) ?
    |> Json.as_string()) ?
    |> println()
  ((root
    |> Json.object_get("active")) ?
    |> Json.as_bool()) ?
    |> print_bool()
  ((root
    |> Json.object_get("count")) ?
    |> Json.as_int()) ?
    |> print_int()
  ((root
    |> Json.object_get("ratio")) ?
    |> Json.as_float()) ?
    |> print_float()
  (root
    |> Json.object_get("empty")) ?
    |> Json.is_null()
    |> print_bool()
  case root
    |> Json.object_get("missing") do
    Ok(_) -> println("unexpected-ok")
    Err(error) -> println(error)
  end
  Ok(0)
end

fn main() do
  case inspect() do
    Ok(_) -> println("done")
    Err(error) -> println(error)
  end
end
