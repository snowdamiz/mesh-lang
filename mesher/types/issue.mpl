# Issue data types for Mesher monitoring platform.
# Issues are grouped errors identified by fingerprint.

module Types.Issue

# Issue lifecycle status -- state machine for issue resolution.
pub type IssueStatus do
  Unresolved | Resolved | Archived
end deriving(Json)

# Database Row struct for issues. Status stored as text in DB,
# parsed to IssueStatus via from_json when needed.
pub struct Issue do
  id :: String
  project_id :: String
  fingerprint :: String
  title :: String
  level :: String
  status :: String
  event_count :: Int
  first_seen :: String
  last_seen :: String
  assigned_to :: Option<String>
end deriving(Json, Row)
