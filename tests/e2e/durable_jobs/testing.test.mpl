from Test import assert_enqueued_in

fn spec() -> String do
  Json.encode(json {
    worker: "proof",
    version: 2,
    queue: "testing",
    args: json { label: "manual", scope: namespace(), nested: json { a: 1, b: 2 }, items: [1, 2] }
  })
end

fn filter() -> String do
  Json.encode(json {
    worker: "proof",
    version: 2,
    queue: "testing",
    state: "available",
    args: json { label: "manual", scope: namespace(), nested: json { a: 1 }, items: [2] }
  })
end

fn namespace() -> String do
  Env.get("MESH_JOBS_PROOF_NAMESPACE", "")
end

fn database_url() -> String do
  Env.get("MESH_TEST_DATABASE_URL", "")
end

fn check(result :: Unit!String, step :: String) -> Unit!String do
  case result do
    Ok(_) -> Ok(())
    Err(error) -> Err(step <> ": " <> error)
  end
end

fn check_borrowed(conn :: borrow PgConn) -> String!String do
  let _ = Jobs.enqueue_in(conn, namespace(), spec())?
  let _ = check(conn
      |> assert_enqueued_in(namespace(), filter()),
    "borrowed match")?
  let _ = check(Test.refute_enqueued_in(conn,
      namespace(),
      """{"worker":"proof","version":1,"args":{"label":"manual"}}"""),
    "other version")?
  let _ = check(Test.refute_enqueued_in(conn, namespace(), """{"args":{"nested":{"a":9}}}"""),
    "other args")?
  let _ = check(Test.refute_enqueued_in(conn,
      namespace(),
      """{"queue":"default","args":{"label":"manual"}}"""),
    "other queue")?
  let _ = check(Test.refute_enqueued_in(conn,
      namespace(),
      """{"state":"executing","args":{"label":"manual"}}"""),
    "other state")?
  let rows = Test.jobs_inspect_in(conn, namespace(), filter())?
  let outside = Pool.open(database_url(), 1, 1, 5000)?
  let visibility = Test.refute_enqueued(outside, namespace(), filter(), 20)
  Pool.close(outside)
  let _ = check(visibility, "outside transaction")?
  if String.contains(rows, "manual") do
    Err("force rollback after borrowed assertion")
  else
    Err("inspection did not return the borrowed row")
  end
end

fn check_manual(pool :: PoolHandle) -> Unit!String do
  case Repo.transaction(pool, check_borrowed) do
    Err(error) ->
      if error != "force rollback after borrowed assertion" do
        return Err(error)
      end
    Ok(_) -> return Err("transaction committed unexpectedly")
  end
  let _ = check(Test.refute_enqueued(pool, namespace(), filter(), 0), "after rollback")?
  let _ = Jobs.enqueue(pool, namespace(), spec())?
  let _ = check(Test.assert_enqueued(pool, namespace(), filter(), 50), "after commit")?
  let other = Env.get("MESH_JOBS_OTHER_NAMESPACE", "")
  let _ = check(Test.refute_enqueued(pool, other, filter(), 0), "other namespace")?
  let missing = Test.assert_enqueued(pool, namespace(), """{"args":{"label":"missing"}}""", 20)
  case missing do
    Err(_) -> Ok(())
    Ok(_) -> Err("assertion accepted missing work")
  end
end

fn field(context :: String, key :: String) -> Json!String do
  Json.object_get(Json.parse(context)?, key)
end

fn run_inline() -> Unit!String do
  let input = """{"worker":"proof","delay_ms":23,"args":{"label":"inline"}}"""
  let first = Test.build_job(input, 1000)?
  let second = Test.build_job(input, 1000)?
  if Json.as_string(field(first, "id")?)? == Json.as_string(field(second, "id")?)? do
    return Err("synthetic attempts reused an id")
  end
  if Json.as_int(field(first, "scheduled_at_ms")?)? != 1023 do
    return Err("injected clock was ignored")
  end
  let expected = """{"action":"snooze","delay_ms":50}"""
  let outcome = Test.perform_job(input,
    1000,
    fn(context) do
      if String.contains(context, "inline") do
        expected
      else
        """{"action":"discard","reason":"missing context"}"""
      end
    end)?
  if outcome != expected do
    return Err("direct handler returned " <> outcome)
  end
  let malformed = Test.perform_job(input, -1, fn(_) do "not JSON" end)?
  if !String.contains(malformed, "invalid job handler outcome JSON") do
    return Err("direct handler did not use the worker outcome decoder")
  end
  let invalid = Test.perform_job("""{"worker":"proof","args":[]}""",
    -1,
    fn(_) do
      println("invalid synthetic handler ran")
      """{"action":"complete"}"""
    end)
  case invalid do
    Err(error) ->
      if String.contains(error, "JSON objects") do
        Ok(())
      else
        Err(error)
      end
    Ok(_) -> Err("direct execution accepted invalid args")
  end
end

test("synthetic jobs use fresh identities, validated input and decoded outcomes") do
  case run_inline() do
    Ok(_) -> assert(true)
    Err(error) -> panic(error)
  end
end

test("borrowed assertions see an enqueue that rolls back outside the transaction") do
  if database_url() != "" do
    case Pool.open(database_url(), 1, 2, 5000) do
      Ok(pool) ->
        let tested = check_manual(pool)
        Pool.close(pool)
        case tested do
          Ok(_) -> assert(true)
          Err(error) -> panic(error)
        end
      Err(error) -> panic(error)
    end
  end
end
