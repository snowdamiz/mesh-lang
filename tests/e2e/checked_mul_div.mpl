fn main() do
  case Checked.mul_div(5, 3, 2, :half_even) do
    Ok(value) -> println("${value}")
    Err(error) -> println(error)
  end

  case Checked.add(20, 22) do
    Ok(value) -> println("${value}")
    Err(error) -> println(error)
  end

  case Checked.sub(20, 22) do
    Ok(value) -> println("${value}")
    Err(error) -> println(error)
  end

  case Checked.mul(-6, 7) do
    Ok(value) -> println("${value}")
    Err(error) -> println(error)
  end

  case Checked.div(-9, 2) do
    Ok(value) -> println("${value}")
    Err(error) -> println(error)
  end

  case Checked.abs(-42) do
    Ok(value) -> println("${value}")
    Err(error) -> println(error)
  end

  case Checked.rescale(12355, 3, 2, :half_even) do
    Ok(value) -> println("${value}")
    Err(error) -> println(error)
  end
end
