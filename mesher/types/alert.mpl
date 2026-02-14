# Alert data types for Mesher monitoring platform.
# Alert rules define conditions and actions for automated notifications.

module Types.Alert

# Alert rule -- condition_json and action_json are JSONB stored as String in Row struct.
pub struct AlertRule do
  id :: String
  project_id :: String
  name :: String
  condition_json :: String
  action_json :: String
  enabled :: Bool
  created_at :: String
end deriving(Json, Row)

# Typed alert condition for JSON parsing -- not a Row struct.
pub struct AlertCondition do
  condition_type :: String
  threshold :: Int
  window_minutes :: Int
end deriving(Json)
