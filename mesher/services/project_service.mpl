# Project service for Mesher monitoring platform.
# Thin actor wrapper around Storage.Queries for project and API key operations.
# Holds the pool handle as state and delegates all database work to queries.

module Services.ProjectService

from Types.Project import { Project }
from Storage.Queries import { insert_project, get_project, list_projects_by_org, create_api_key, get_project_by_api_key, revoke_api_key }

service ProjectService do
  fn init(pool :: Int) -> Int do
    pool
  end

  call CreateProject(org_id :: String, name :: String, platform :: String) :: String!String do |pool|
    let result = insert_project(pool, org_id, name, platform)
    (result, pool)
  end

  call GetProject(id :: String) :: Project!String do |pool|
    let result = get_project(pool, id)
    (result, pool)
  end

  call ListProjectsByOrg(org_id :: String) :: List<Project>!String do |pool|
    let result = list_projects_by_org(pool, org_id)
    (result, pool)
  end

  call CreateApiKey(project_id :: String, label :: String) :: String!String do |pool|
    let result = create_api_key(pool, project_id, label)
    (result, pool)
  end

  call GetProjectByApiKey(key_value :: String) :: Project!String do |pool|
    let result = get_project_by_api_key(pool, key_value)
    (result, pool)
  end

  call RevokeApiKey(key_id :: String) :: Int!String do |pool|
    let result = revoke_api_key(pool, key_id)
    (result, pool)
  end
end
