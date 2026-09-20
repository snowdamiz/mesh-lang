fn print_u64(value :: U64) do
  value |> U64.to_string() |> println()
end

fn print_u128(value :: U128) do
  value |> U128.to_string() |> println()
end

fn print_i128(value :: I128) do
  value |> I128.to_string() |> println()
end

fn print_int(value :: Int) do
  println("#{value}")
end

fn subtract_u64() -> U64 ! String do
  U64.parse("10") ? |> U64.subtract(U64.parse("3") ?)
end

fn overflow_u64() -> U64 ! String do
  U64.parse("18446744073709551615") ? |> U64.add(U64.parse("1") ?)
end

fn max_u64_to_int() -> Int ! String do
  U64.parse("18446744073709551615") ? |> U64.to_int()
end

fn add_u128() -> U128 ! String do
  U128.parse("340282366920938463463374607431768211450") ?
    |> U128.add(U128.parse("5") ?)
end

fn underflow_u128() -> U128 ! String do
  U128.parse("0") ? |> U128.subtract(U128.parse("1") ?)
end

fn compare_u128() -> Int ! String do
  Ok(U128.compare(U128.parse("1") ?, U128.parse("2") ?))
end

fn nav_u128() -> U128 ! String do
  (U128.parse("12345678900") ?
    |> U128.multiply(U128.parse("1000000000") ?)) ?
    |> U128.divide(U128.parse("10000000000") ?)
end

fn divide_u128_by_zero() -> U128 ! String do
  U128.parse("1") ? |> U128.divide(U128.parse("0") ?)
end

fn subtract_i128() -> I128 ! String do
  I128.parse("-40") ? |> I128.subtract(I128.parse("2") ?)
end

fn overflow_i128() -> I128 ! String do
  I128.parse("170141183460469231731687303715884105727") ?
    |> I128.add(I128.parse("1") ?)
end

fn negative_i128_to_int() -> Int ! String do
  I128.parse("-42") ? |> I128.to_int()
end

fn main() do
  case U64.parse("18446744073709551615") do
    Ok(value) -> print_u64(value)
    Err(error) -> println(error)
  end
  case subtract_u64() do
    Ok(value) -> print_u64(value)
    Err(error) -> println(error)
  end
  case overflow_u64() do
    Ok(value) -> print_u64(value)
    Err(error) -> println(error)
  end
  case max_u64_to_int() do
    Ok(value) -> print_int(value)
    Err(error) -> println(error)
  end

  case add_u128() do
    Ok(value) -> print_u128(value)
    Err(error) -> println(error)
  end
  case underflow_u128() do
    Ok(value) -> print_u128(value)
    Err(error) -> println(error)
  end
  case compare_u128() do
    Ok(value) -> print_int(value)
    Err(error) -> println(error)
  end
  case nav_u128() do
    Ok(value) -> print_u128(value)
    Err(error) -> println(error)
  end
  case divide_u128_by_zero() do
    Ok(value) -> print_u128(value)
    Err(error) -> println(error)
  end

  case I128.parse("-170141183460469231731687303715884105728") do
    Ok(value) -> print_i128(value)
    Err(error) -> println(error)
  end
  case subtract_i128() do
    Ok(value) -> print_i128(value)
    Err(error) -> println(error)
  end
  case overflow_i128() do
    Ok(value) -> print_i128(value)
    Err(error) -> println(error)
  end
  case negative_i128_to_int() do
    Ok(value) -> print_int(value)
    Err(error) -> println(error)
  end
end
