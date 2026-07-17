fn select_header(headers :: List<String>) -> Option<String> do
  let normalized = List.find(headers, fn(value) -> value == "authorization" end)
  case normalized do
    Some(value) -> Some(value)
    None -> List.find(headers, fn(value) -> value == "Authorization" end)
  end
end

fn print_selected(headers :: List<String>) do
  case select_header(headers) do
    Some(value) -> println(value)
    None -> println("missing")
  end
end

fn select_header_with_if(headers :: List<String>, fallback :: Bool) -> Option<String> do
  if fallback do
    List.find(headers, fn(value) -> value == "Authorization" end)
  else
    Some("local")
  end
end

fn print_selected_with_if(headers :: List<String>, fallback :: Bool) do
  case select_header_with_if(headers, fallback) do
    Some(value) -> println(value)
    None -> println("missing")
  end
end

fn main() do
  print_selected(["authorization"])
  print_selected(["Authorization"])
  print_selected(["content-type"])
  print_selected_with_if(["authorization"], false)
  print_selected_with_if(["Authorization"], true)
end
