struct Point do
  x :: Int
end

fn show(point :: Point) do
  point.x
    |> Int.to_string()
    |> println()
end

fn main() do
  [Point { x : 7 }]
    |> List.get(0)
    |> show()
end
