@export("mesh_fixture_echo")
pub fn echo(request :: Bytes) -> Bytes ! String do
  Host.secure_store_get(request)
end
