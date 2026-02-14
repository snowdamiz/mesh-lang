# Project and organization data types for Mesher monitoring platform.
# Flat org -> project hierarchy (no teams layer).

module Types.Project

# Organization -- top-level tenancy unit.
pub struct Organization do
  id :: String
  name :: String
  slug :: String
  created_at :: String
end deriving(Json, Row)

# Project -- belongs to an organization, receives events.
pub struct Project do
  id :: String
  org_id :: String
  name :: String
  platform :: Option<String>
  created_at :: String
end deriving(Json, Row)

# API key -- multiple keys per project for rotation and environment separation.
# Key format: mshr_ prefix with hex-encoded random bytes.
pub struct ApiKey do
  id :: String
  project_id :: String
  key_value :: String
  label :: String
  created_at :: String
  revoked_at :: Option<String>
end deriving(Json, Row)
