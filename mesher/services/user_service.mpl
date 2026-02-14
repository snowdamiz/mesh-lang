# User service for Mesher monitoring platform.
# Thin actor wrapper around Storage.Queries for user auth, session, and membership.
# Holds the pool handle as state and delegates all database work to queries.
# Login is a two-step operation: authenticate credentials, then create session.

module Services.UserService

from Types.User import { User, Session, OrgMembership }
from Storage.Queries import { create_user, authenticate_user, get_user, create_session, validate_session, delete_session, add_member, get_members }

service UserService do
  fn init(pool :: Int) -> Int do
    pool
  end

  call Register(email :: String, password :: String, display_name :: String) :: String!String do |pool|
    let result = create_user(pool, email, password, display_name)
    (result, pool)
  end

  call Login(email :: String, password :: String) :: String!String do |pool|
    let auth_result = authenticate_user(pool, email, password)
    let token = case auth_result do
      Ok(user) -> create_session(pool, user.id)
      Err(e) -> Err(e)
    end
    (token, pool)
  end

  call ValidateSession(token :: String) :: Session!String do |pool|
    let result = validate_session(pool, token)
    (result, pool)
  end

  call Logout(token :: String) :: Int!String do |pool|
    let result = delete_session(pool, token)
    (result, pool)
  end

  call GetUser(id :: String) :: User!String do |pool|
    let result = get_user(pool, id)
    (result, pool)
  end

  call AddMember(user_id :: String, org_id :: String, role :: String) :: String!String do |pool|
    let result = add_member(pool, user_id, org_id, role)
    (result, pool)
  end

  call GetMembers(org_id :: String) :: List<OrgMembership>!String do |pool|
    let result = get_members(pool, org_id)
    (result, pool)
  end
end
