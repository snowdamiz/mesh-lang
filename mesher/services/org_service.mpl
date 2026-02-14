# Organization service for Mesher monitoring platform.
# Thin actor wrapper around Storage.Queries for org CRUD operations.
# Holds the pool handle as state and delegates all database work to queries.

module Services.OrgService

from Types.Project import { Organization }
from Storage.Queries import { insert_org, get_org, list_orgs }

service OrgService do
  fn init(pool :: Int) -> Int do
    pool
  end

  call CreateOrg(name :: String, slug :: String) :: String!String do |pool|
    let result = insert_org(pool, name, slug)
    (result, pool)
  end

  call GetOrg(id :: String) :: Organization!String do |pool|
    let result = get_org(pool, id)
    (result, pool)
  end

  call ListOrgs() :: List<Organization>!String do |pool|
    let result = list_orgs(pool)
    (result, pool)
  end
end
