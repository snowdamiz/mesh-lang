##! Bounded public GF(2^16) systematic erasure words. Not authentication.
pub type ErasureError do
  InvalidParameters
  MalformedChunk
  ConflictingDuplicate
  InconsistentCodewords
end deriving(Eq, Debug)

struct ErasureChunk do
  index :: Int
  payload :: Bytes
end

# Polynomial basis, modulus x^16 + x^12 + x^3 + x + 1 (0x1100b).
# Mesh Int has no bitwise operator. These fixed arithmetic expressions implement
# XOR without a lookup table, allocation, recursion or host intrinsic.
fn xor_nibble(left :: Int, right :: Int) -> Int do
  (left % 2 + right % 2) % 2
    + 2 * ((left / 2 % 2 + right / 2 % 2) % 2)
    + 4 * ((left / 4 % 2 + right / 4 % 2) % 2)
    + 8 * ((left / 8 % 2 + right / 8 % 2) % 2)
end

fn field_add(left :: Int, right :: Int) -> Int do
  xor_nibble(left % 16, right % 16)
    + 16 * xor_nibble(left / 16 % 16, right / 16 % 16)
    + 256 * xor_nibble(left / 256 % 16, right / 256 % 16)
    + 4096 * xor_nibble(left / 4096, right / 4096)
end

fn field_double(value :: Int) -> Int do
  if value < 32768 do
    value * 2
  else
    let reduced = value * 2 - 65536
    # Toggle the low polynomial 0x100b; reduced is even, so bit zero is zero.
    reduced
      + 4096 * (1 - 2 * (reduced / 4096 % 2))
      + 8 * (1 - 2 * (reduced / 8 % 2))
      + 2 * (1 - 2 * (reduced / 2 % 2))
      + 1
  end
end

# Exactly sixteen polynomial steps, all scalar locals; no field tables.
fn field_multiply(left :: Int, right :: Int) -> Int do
  let word_0 = left
  let sum_0 = if right / 1 % 2 == 0 do
    0
  else
    word_0
  end
  let word_1 = field_double(word_0)
  let sum_1 = if right / 2 % 2 == 0 do
    sum_0
  else
    field_add(sum_0, word_1)
  end
  let word_2 = field_double(word_1)
  let sum_2 = if right / 4 % 2 == 0 do
    sum_1
  else
    field_add(sum_1, word_2)
  end
  let word_3 = field_double(word_2)
  let sum_3 = if right / 8 % 2 == 0 do
    sum_2
  else
    field_add(sum_2, word_3)
  end
  let word_4 = field_double(word_3)
  let sum_4 = if right / 16 % 2 == 0 do
    sum_3
  else
    field_add(sum_3, word_4)
  end
  let word_5 = field_double(word_4)
  let sum_5 = if right / 32 % 2 == 0 do
    sum_4
  else
    field_add(sum_4, word_5)
  end
  let word_6 = field_double(word_5)
  let sum_6 = if right / 64 % 2 == 0 do
    sum_5
  else
    field_add(sum_5, word_6)
  end
  let word_7 = field_double(word_6)
  let sum_7 = if right / 128 % 2 == 0 do
    sum_6
  else
    field_add(sum_6, word_7)
  end
  let word_8 = field_double(word_7)
  let sum_8 = if right / 256 % 2 == 0 do
    sum_7
  else
    field_add(sum_7, word_8)
  end
  let word_9 = field_double(word_8)
  let sum_9 = if right / 512 % 2 == 0 do
    sum_8
  else
    field_add(sum_8, word_9)
  end
  let word_10 = field_double(word_9)
  let sum_10 = if right / 1024 % 2 == 0 do
    sum_9
  else
    field_add(sum_9, word_10)
  end
  let word_11 = field_double(word_10)
  let sum_11 = if right / 2048 % 2 == 0 do
    sum_10
  else
    field_add(sum_10, word_11)
  end
  let word_12 = field_double(word_11)
  let sum_12 = if right / 4096 % 2 == 0 do
    sum_11
  else
    field_add(sum_11, word_12)
  end
  let word_13 = field_double(word_12)
  let sum_13 = if right / 8192 % 2 == 0 do
    sum_12
  else
    field_add(sum_12, word_13)
  end
  let word_14 = field_double(word_13)
  let sum_14 = if right / 16384 % 2 == 0 do
    sum_13
  else
    field_add(sum_13, word_14)
  end
  let word_15 = field_double(word_14)
  let sum_15 = if right / 32768 % 2 == 0 do
    sum_14
  else
    field_add(sum_14, word_15)
  end
  sum_15
end

# Distinct validated points make each denominator nonzero. Fermat exponent
# 65534 uses fifteen fixed square/multiply steps, independent of input values.
fn field_inverse(value :: Int) -> Int do
  let power_0 = value
  let power_1 = field_multiply(field_multiply(power_0, power_0), value)
  let power_2 = field_multiply(field_multiply(power_1, power_1), value)
  let power_3 = field_multiply(field_multiply(power_2, power_2), value)
  let power_4 = field_multiply(field_multiply(power_3, power_3), value)
  let power_5 = field_multiply(field_multiply(power_4, power_4), value)
  let power_6 = field_multiply(field_multiply(power_5, power_5), value)
  let power_7 = field_multiply(field_multiply(power_6, power_6), value)
  let power_8 = field_multiply(field_multiply(power_7, power_7), value)
  let power_9 = field_multiply(field_multiply(power_8, power_8), value)
  let power_10 = field_multiply(field_multiply(power_9, power_9), value)
  let power_11 = field_multiply(field_multiply(power_10, power_10), value)
  let power_12 = field_multiply(field_multiply(power_11, power_11), value)
  let power_13 = field_multiply(field_multiply(power_12, power_12), value)
  let power_14 = field_multiply(field_multiply(power_13, power_13), value)
  field_multiply(power_14, power_14)
end

fn valid_parameters(size :: Int, width :: Int) -> Bool do
  size >= 1 && size <= 1152 && width >= 32 && width <= 256 && width % 2 == 0
end

fn bytes_slice(input :: Bytes, offset :: Int, count :: Int) -> Bytes!ErasureError do
  case Bytes.slice(input, offset, count) do
    Err(_) -> Err(MalformedChunk)
    Ok(value)
  end
end

fn join_words(words :: List<Bytes>) -> Bytes!ErasureError do
  case Bytes.concat_many(words, 1380) do
    Err(_) -> Err(MalformedChunk)
    Ok(value)
  end
end

fn padded_source(message :: Bytes, width :: Int) -> Bytes!ErasureError do
  let k = (Bytes.length(message) + width - 1) / width
  let zeros = case Bytes.repeat(0, k * width - Bytes.length(message)) do
    Err(_) -> return Err(InvalidParameters)
    Ok(value) -> value
  end
  join_words([message, zeros])
end

fn fields(words :: List<Bytes>, width :: Int) -> List<List<Int>>!ErasureError do
  let rows = for word in words do
    for column in 0..(width / 2) do
      case Bytes.read_u16_be(word, column * 2) do
        Err(_) -> return Err(MalformedChunk)
        Ok(value) -> value
      end
    end
  end
  Ok(rows)
end

fn inverse_denominators(points :: List<Int>) -> List<Int> do
  let count = List.length(points)
  for row in 0..count do
    let product = List.reduce(Range.to_list(0..count),
      1,
      fn value, column -> if row == column do
        value
      else
        field_multiply(value, field_add(List.get(points, row), List.get(points, column)))
      end end)
    field_inverse(product)
  end
end

# Prefix/suffix products give all Lagrange weights in O(k) scalar operations,
# including evaluation at an existing point, without division by x + point.
fn weights(points :: List<Int>, inverses :: List<Int>, index :: Int) -> List<Int> do
  let prefix = List.reduce(points,
    [1],
    fn values, point -> List.append(values,
      field_multiply(List.last(values), field_add(index, point))) end)
  let backwards = List.reduce(List.reverse(points),
    [1],
    fn values, point -> List.append(values,
      field_multiply(List.last(values), field_add(index, point))) end)
  let suffix = List.reverse(backwards)
  for row in 0..List.length(points) do
    field_multiply(field_multiply(List.get(prefix, row), List.get(suffix, row + 1)),
      List.get(inverses, row))
  end
end

fn evaluate(rows :: List<List<Int>>,
  points :: List<Int>,
  inverses :: List<Int>,
  width :: Int,
  index :: Int) -> Bytes!ErasureError do
  let factors = weights(points, inverses, index)
  let builder = case BytesBuilder.new(width) do
    Err(_) -> return Err(MalformedChunk)
    Ok(value) -> value
  end
  for column in 0..(width / 2) do
    let word = List.reduce(Range.to_list(0..List.length(points)),
      0,
      fn value, row -> field_add(value,
        field_multiply(List.get(factors, row), List.get(List.get(rows, row), column))) end)
    case BytesBuilder.write_u16_be(builder, word) do
      Err(_) -> return Err(MalformedChunk)
      Ok(_) -> nil
    end
  end
  case BytesBuilder.finish(builder) do
    Err(_) -> Err(MalformedChunk)
    Ok(value)
  end
end

pub fn encode_erasure_chunk(message :: Bytes, width :: Int, index :: Int) -> Bytes!ErasureError do
  let size = Bytes.length(message)
  if !valid_parameters(size, width) || index < 0 || index > 65535 do
    return Err(InvalidParameters)
  end
  let k = (size + width - 1) / width
  let padded = padded_source(message, width)?
  if index < k do
    return bytes_slice(padded, index * width, width)
  end
  let points = Range.to_list(0..k)
  let words = for row in 0..k do
    bytes_slice(padded, row * width, width)?
  end
  evaluate(fields(words, width)?, points, inverse_denominators(points), width, index)
end

fn validate_entries(width :: Int,
  indices :: List<Int>,
  payloads :: List<Bytes>) -> Unit!ErasureError do
  let count = List.length(indices)
  if count != List.length(payloads) || count > 72 do
    return Err(MalformedChunk)
  end
  for row in 0..count do
    let index = List.get(indices, row)
    if index < 0 || index > 65535 || Bytes.length(List.get(payloads, row)) != width do
      return Err(MalformedChunk)
    end
  end
  Ok(nil)
end

fn append_unique(state :: List<ErasureChunk>!ErasureError,
  entry :: ErasureChunk) -> List<ErasureChunk>!ErasureError do
  let values = state?
  case List.find(values, fn previous -> previous.index == entry.index end) do
    None -> Ok(List.append(values, entry))
    Some(previous) -> if Bytes.secure_equals(previous.payload, entry.payload) do
      Ok(values)
    else
      Err(ConflictingDuplicate)
    end
  end
end

fn deduplicate(indices :: List<Int>, payloads :: List<Bytes>) -> List<ErasureChunk>!ErasureError do
  List.reduce(Range.to_list(0..List.length(indices)),
    Ok([]),
    fn state, row -> append_unique(state,
      ErasureChunk { index: List.get(indices, row), payload: List.get(payloads, row) }) end)
end

fn systematic_word(selected :: List<ErasureChunk>, index :: Int) -> Bytes!ErasureError do
  case List.find(selected, fn entry -> entry.index == index end) do
    None -> Err(InconsistentCodewords)
    Some(entry) -> Ok(entry.payload)
  end
end

fn reconstructed_words(entries :: List<ErasureChunk>,
  width :: Int,
  k :: Int) -> List<Bytes>!ErasureError do
  let selected = List.take(entries, k)
  let systematic = List.all(selected, fn entry -> entry.index < k end)
  if systematic && List.length(entries) == k do
    Ok(for index in 0..k do
      systematic_word(selected, index)?
    end)
  else
    let points = for entry in selected do
      entry.index
    end
    let words = for entry in selected do
      entry.payload
    end
    let rows = fields(words, width)?
    let inverses = inverse_denominators(points)
    let restored = for index in 0..k do
      case List.find(selected, fn entry -> entry.index == index end) do
        Some(entry) -> entry.payload
        None -> evaluate(rows, points, inverses, width, index)?
      end
    end
    for entry in List.drop(entries, k) do
      if !Bytes.secure_equals(evaluate(rows, points, inverses, width, entry.index)?,
        entry.payload) do
        return Err(InconsistentCodewords)
      end
    end
    Ok(restored)
  end
end

fn unpad(words :: List<Bytes>, size :: Int) -> Bytes!ErasureError do
  let padded = join_words(words)?
  for offset in size..Bytes.length(padded) do
    case Bytes.get(padded, offset) do
      Err(_) -> return Err(MalformedChunk)
      Ok(value) -> if value != 0 do
        return Err(InconsistentCodewords)
      end
    end
  end
  bytes_slice(padded, 0, size)
end

pub fn decode_erasure_chunks(message_bytes :: Int,
  width :: Int,
  indices :: List<Int>,
  payloads :: List<Bytes>) -> Option<Bytes>!ErasureError do
  if !valid_parameters(message_bytes, width) do
    return Err(InvalidParameters)
  end
  validate_entries(width, indices, payloads)?
  let entries = deduplicate(indices, payloads)?
  let k = (message_bytes + width - 1) / width
  if List.length(entries) < k do
    Ok(None)
  else
    Ok(Some(unpad(reconstructed_words(entries, width, k)?, message_bytes)?))
  end
end
