# User and authentication data types for Mesher monitoring platform.
# NO password_hash field in the User struct -- never expose to application code.

module Types.User

# User account -- password_hash lives only in the database.
pub struct User do
  id :: String
  email :: String
  display_name :: String
  created_at :: String
end deriving(Json, Row)

# Organization membership -- many-to-many with role (owner/admin/member).
pub struct OrgMembership do
  id :: String
  user_id :: String
  org_id :: String
  role :: String
  joined_at :: String
end deriving(Json, Row)

# Session token -- opaque 64-char hex, not JWT.
pub struct Session do
  token :: String
  user_id :: String
  created_at :: String
  expires_at :: String
end deriving(Json, Row)
