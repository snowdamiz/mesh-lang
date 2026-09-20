@native("mesh_borsh_reader")
pub fn reader(bytes :: Bytes, max_collection :: Int) -> Int!String

@native("mesh_borsh_remaining")
pub fn remaining(reader :: Int) -> Int!String

@native("mesh_borsh_finish_reader")
pub fn finish_reader(reader :: Int) -> Int!String

@native("mesh_borsh_close_reader")
pub fn close_reader(reader :: Int) -> Int

@native("mesh_borsh_read_u8")
pub fn read_u8(reader :: Int) -> Int!String

@native("mesh_borsh_read_i8")
pub fn read_i8(reader :: Int) -> Int!String

@native("mesh_borsh_read_u16")
pub fn read_u16(reader :: Int) -> Int!String

@native("mesh_borsh_read_i16")
pub fn read_i16(reader :: Int) -> Int!String

@native("mesh_borsh_read_u32")
pub fn read_u32(reader :: Int) -> Int!String

@native("mesh_borsh_read_i32")
pub fn read_i32(reader :: Int) -> Int!String

@native("mesh_borsh_read_u64")
pub fn read_u64(reader :: Int) -> U64!String

@native("mesh_borsh_read_i64")
pub fn read_i64(reader :: Int) -> I128!String

@native("mesh_borsh_read_u128")
pub fn read_u128(reader :: Int) -> U128!String

@native("mesh_borsh_read_i128")
pub fn read_i128(reader :: Int) -> I128!String

@native("mesh_borsh_read_bool")
pub fn read_bool(reader :: Int) -> Bool!String

@native("mesh_borsh_read_fixed")
pub fn read_fixed(reader :: Int, length :: Int) -> Bytes!String

@native("mesh_borsh_read_len")
pub fn read_len(reader :: Int) -> Int!String

@native("mesh_borsh_read_vec")
pub fn read_vec(reader :: Int) -> Bytes!String

@native("mesh_borsh_read_string")
pub fn read_string(reader :: Int) -> String!String

@native("mesh_borsh_read_option_tag")
pub fn read_option_tag(reader :: Int) -> Bool!String

@native("mesh_borsh_writer")
pub fn writer(max_output :: Int) -> Int!String

@native("mesh_borsh_finish_writer")
pub fn finish_writer(writer :: Int) -> Bytes!String

@native("mesh_borsh_close_writer")
pub fn close_writer(writer :: Int) -> Int

@native("mesh_borsh_write_u8")
pub fn write_u8(writer :: Int, value :: Int) -> Int!String

@native("mesh_borsh_write_i8")
pub fn write_i8(writer :: Int, value :: Int) -> Int!String

@native("mesh_borsh_write_u16")
pub fn write_u16(writer :: Int, value :: Int) -> Int!String

@native("mesh_borsh_write_i16")
pub fn write_i16(writer :: Int, value :: Int) -> Int!String

@native("mesh_borsh_write_u32")
pub fn write_u32(writer :: Int, value :: Int) -> Int!String

@native("mesh_borsh_write_i32")
pub fn write_i32(writer :: Int, value :: Int) -> Int!String

@native("mesh_borsh_write_u64")
pub fn write_u64(writer :: Int, value :: U64) -> Int!String

@native("mesh_borsh_write_i64")
pub fn write_i64(writer :: Int, value :: I128) -> Int!String

@native("mesh_borsh_write_u128")
pub fn write_u128(writer :: Int, value :: U128) -> Int!String

@native("mesh_borsh_write_i128")
pub fn write_i128(writer :: Int, value :: I128) -> Int!String

@native("mesh_borsh_write_bool")
pub fn write_bool(writer :: Int, value :: Bool) -> Int!String

@native("mesh_borsh_write_fixed")
pub fn write_fixed(writer :: Int, value :: Bytes) -> Int!String

@native("mesh_borsh_write_len")
pub fn write_len(writer :: Int, length :: Int) -> Int!String

@native("mesh_borsh_write_vec")
pub fn write_vec(writer :: Int, value :: Bytes) -> Int!String

@native("mesh_borsh_write_string")
pub fn write_string(writer :: Int, value :: String) -> Int!String

@native("mesh_borsh_write_option_tag")
pub fn write_option_tag(writer :: Int, present :: Bool) -> Int!String
