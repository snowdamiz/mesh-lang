fn proof() -> Bool!String do
  let a = U64.parse("18446744073709551615")?
  println(U64.to_string(U64.multiply_divide(a, a, a)?))
  let b = U128.parse("340282366920938463463374607431768211455")?
  println(U128.to_string(U128.multiply_divide(b, b, b)?))
  println(U128.to_string(U128.multiply_divide(U128.parse("7")?, U128.parse("9")?, U128.parse("4")?)?))
  case U128.multiply_divide(b, b, U128.subtract(b, U128.parse("1")?)?) do
    Ok(_) -> println("unexpected overflow success")
    Err(error) -> println(error)
  end
  case U64.multiply_divide(a, a, U64.parse("0")?) do
    Ok(_) -> println("unexpected division success")
    Err(error) -> println(error)
  end
  Ok(true)
end

fn main() do
  case proof() do
    Ok(_) -> nil
    Err(error) -> println(error)
  end
end
