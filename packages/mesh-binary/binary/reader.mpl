pub type BinaryError do
  InvalidLimit
  InputTooLarge
  InvalidLength
  UnexpectedEnd
  VectorTooLarge
  TrailingBytes
end deriving(Eq, Debug)

pub struct BinaryReader do
  input :: Bytes
  offset :: Int
  maximum :: Int
end

fn remaining(state :: BinaryReader) -> Int ! BinaryError do
  let length = Bytes.length(state.input)
  if state.maximum < 0 || state.offset < 0 do
    Err(InvalidLength)
  else
    if length > state.maximum || state.offset > length || state.offset > state.maximum do
      Err(InvalidLength)
    else
      Ok(length - state.offset)
    end
  end
end

fn advance(state :: BinaryReader, count :: Int) -> BinaryReader do
  BinaryReader {
    input: state.input,
    offset: state.offset + count,
    maximum: state.maximum
  }
end

pub fn reader(input :: Bytes, maximum :: Int) -> BinaryReader ! BinaryError do
  if maximum < 0 do
    Err(InvalidLimit)
  else
    if Bytes.length(input) > maximum do
      Err(InputTooLarge)
    else
      Ok(BinaryReader { input: input, offset: 0, maximum: maximum })
    end
  end
end

pub fn read_u8(state :: BinaryReader) -> Result<(BinaryReader, Int), BinaryError> do
  if remaining(state) ? < 1 do
    Err(UnexpectedEnd)
  else
    case Bytes.get(state.input, state.offset) do
      Err(_) -> Err(UnexpectedEnd)
      Ok(value) -> Ok((advance(state, 1), value))
    end
  end
end

pub fn read_u16_be(state :: BinaryReader) -> Result<(BinaryReader, Int), BinaryError> do
  if remaining(state) ? < 2 do
    Err(UnexpectedEnd)
  else
    case Bytes.read_u16_be(state.input, state.offset) do
      Err(_) -> Err(UnexpectedEnd)
      Ok(value) -> Ok((advance(state, 2), value))
    end
  end
end

pub fn read_fixed(
  state :: BinaryReader,
  length :: Int
) -> Result<(BinaryReader, Bytes), BinaryError> do
  if length < 0 do
    Err(InvalidLength)
  else
    if length > remaining(state) ? do
      Err(UnexpectedEnd)
    else
      case Bytes.slice(state.input, state.offset, length) do
        Err(_) -> Err(UnexpectedEnd)
        Ok(value) -> Ok((advance(state, length), value))
      end
    end
  end
end

# Vectors use a canonical unsigned 32-bit big-endian length prefix.
pub fn read_vector(
  state :: BinaryReader,
  maximum :: Int
) -> Result<(BinaryReader, Bytes), BinaryError> do
  if maximum < 0 do
    Err(InvalidLimit)
  else
    if remaining(state) ? < 4 do
      Err(UnexpectedEnd)
    else
      case Bytes.read_u32_be(state.input, state.offset) do
        Err(_) -> Err(UnexpectedEnd)
        Ok(encoded_length) -> case U64.to_int(encoded_length) do
            Err(_) -> Err(VectorTooLarge)
            Ok(length) -> if length > maximum do
                Err(VectorTooLarge)
              else
                read_fixed(advance(state, 4), length)
              end
          end
      end
    end
  end
end

pub fn finish(state :: BinaryReader) -> Result<(), BinaryError> do
  if remaining(state) ? == 0 do
    Ok(nil)
  else
    Err(TrailingBytes)
  end
end
