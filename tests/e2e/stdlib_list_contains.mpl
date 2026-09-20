fn main() do
  let nums = [10, 20, 30, 40]
  println("${List.contains(nums, 20)}")
  println("${List.contains(nums, 50)}")
  let empty = List.new()
  println("${List.contains(empty, 1)}")
  println("${List.contains(["paper", "shadow"], "paper") == true}")
  println("${List.contains(["paper", "shadow"], "live") == false}")
end
