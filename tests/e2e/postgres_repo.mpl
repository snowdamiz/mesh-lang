struct Author do
  table "writers"
  primary_key :handle
  handle :: String
  name :: String
  has_many :posts, Post
  has_one :profile, Profile
end deriving(Schema)

struct Post do
  table "articles"
  id :: String
  author_id :: String
  title :: String
  belongs_to :author, Author
  has_many :comments, Comment
end deriving(Schema)

struct Comment do
  id :: String
  post_id :: String
  body :: String
  belongs_to :post, Post
end deriving(Schema)

struct Profile do
  id :: String
  author_id :: String
  bio :: String
end deriving(Schema)

fn show(label :: String, result :: Result<Map<String, String>, String>, field :: String) do
  case result do
    Ok(row) -> println(label <> ":" <> Map.get(row, field))
    Err(error) -> println(label <> ":error:" <> error)
  end
end

fn show_rows(label :: String, result :: Result<List<Map<String, String>>, String>, field :: String) do
  case result do
    Ok(rows) -> println(label <> ":" <> String.join(List.map(rows,
        fn(row) do Map.get(row, field) end),
      ","))
    Err(error) -> println(label <> ":error:" <> error)
  end
end

fn show_fields(label :: String, result :: Result<List<Map<String, String>>, String>, fields :: List<String>) do
  case result do
    Ok(rows) -> println(label <> ":" <> String.join(List.map(fields,
        fn(field) do field <> "=" <> Map.get(List.head(rows), field) end),
      " "))
    Err(error) -> println(label <> ":error:" <> error)
  end
end

fn show_int(label :: String, result :: Result<Int, String>) do
  case result do
    Ok(n) -> println(label <> ":" <> String.from(n))
    Err(error) -> println(label <> ":error:" <> error)
  end
end

fn show_bool(label :: String, result :: Result<Bool, String>) do
  case result do
    Ok(b) -> println(label <> ":" <> String.from(b))
    Err(error) -> println(label <> ":error:" <> error)
  end
end

fn failed<T>(label :: String, result :: Result<T, String>) do
  case result do
    Ok(_) -> println(label <> ":unexpected-ok")
    Err(_) -> println(label <> ":failed")
  end
end

fn setup(pool :: PoolHandle) -> Int!String do
  let _ = Pool.execute(pool, "DROP SCHEMA IF EXISTS mesh_repo_e2e CASCADE", [])?
  let _ = Pool.execute(pool, "CREATE SCHEMA mesh_repo_e2e", [])?
  let _ = Pool.execute(pool, "SET search_path TO mesh_repo_e2e", [])?
  let _ = Pool.execute(pool,
    "CREATE TABLE writers (handle TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, score INT NOT NULL DEFAULT 0, nickname TEXT)",
    [])?
  let _ = Pool.execute(pool,
    "CREATE TABLE articles (id SERIAL PRIMARY KEY, author_id TEXT NOT NULL REFERENCES writers(handle), title TEXT NOT NULL, views INT NOT NULL DEFAULT 0)",
    [])?
  let _ = Pool.execute(pool,
    "CREATE TABLE comments (id SERIAL PRIMARY KEY, post_id INT NOT NULL REFERENCES articles(id), body TEXT NOT NULL)",
    [])?
  let _ = Pool.execute(pool,
    "CREATE TABLE profiles (id SERIAL PRIMARY KEY, author_id TEXT NOT NULL, bio TEXT NOT NULL)",
    [])?
  Ok(0)
end

fn writes(pool :: PoolHandle) do
  show("insert",
    Repo.insert(pool, "writers", %{"handle" => "ada", "name" => "Ada", "score" => "3"}),
    "name")
  show("insert_expr",
    Repo.insert_expr(pool,
      "writers",
      %{
        "handle" => Expr.value("bob"),
        "name" => Expr.call("upper", [Expr.value("bob")]),
        "score" => Expr.value("2")
      }),
    "name")
  let _ = Repo.insert(pool,
    "writers",
    %{"handle" => "cy", "name" => "Cy", "score" => "9", "nickname" => "c"})
  failed("insert_duplicate", Repo.insert(pool, "writers", %{"handle" => "ada", "name" => "Ada2"}))
  failed("insert_expr_bad_table",
    Repo.insert_expr(pool, "no_such_table", %{"x" => Expr.value("1")}))
  show("update", Repo.update(pool, "writers", "ada", %{"score" => "4"}), "score")
  failed("update_missing_column", Repo.update(pool, "writers", "ada", %{"no_such_column" => "4"}))
  show("update_where",
    Repo.update_where(pool,
      "writers",
      %{"nickname" => "b"},
      Query.from("writers")
        |> Query.where(:handle, "bob")),
    "nickname")
  show("update_where_expr",
    Repo.update_where_expr(pool,
      "writers",
      %{"score" => Expr.mul(Expr.column("score"), Expr.value("10"))},
      Query.from("writers")
        |> Query.where(:handle, "cy")),
    "score")
  failed("update_where_expr_bad",
    Repo.update_where_expr(pool, "writers", %{"nope" => Expr.value("1")}, Query.from("writers")))
  show("upsert_insert",
    Repo.insert_or_update(pool,
      "writers",
      %{"handle" => "dee", "name" => "Dee", "score" => "1"},
      ["handle"],
      ["score"]),
    "score")
  show("upsert_update",
    Repo.insert_or_update(pool,
      "writers",
      %{"handle" => "dee", "name" => "Dee", "score" => "5"},
      ["handle"],
      ["score"]),
    "score")
  show("upsert_expr",
    Repo.insert_or_update_expr(pool,
      "writers",
      %{"handle" => "dee", "name" => "Dee", "score" => "7"},
      ["handle"],
      %{"score" => Expr.add(Expr.column("score"), Expr.excluded("score"))}),
    "score")
  failed("upsert_bad_conflict",
    Repo.insert_or_update(pool,
      "writers",
      %{"handle" => "eve", "name" => "Eve"},
      ["score"],
      ["name"]))
  let _ = Repo.insert(pool,
    "articles",
    %{"author_id" => "ada", "title" => "First", "views" => "10"})
  let _ = Repo.insert(pool,
    "articles",
    %{"author_id" => "ada", "title" => "Second", "views" => "30"})
  let _ = Repo.insert(pool,
    "articles",
    %{"author_id" => "bob", "title" => "Third", "views" => "20"})
  let _ = Repo.insert(pool, "comments", %{"post_id" => "1", "body" => "nice"})
  let _ = Repo.insert(pool, "comments", %{"post_id" => "1", "body" => "great"})
  let _ = Repo.insert(pool, "comments", %{"post_id" => "3", "body" => "ok"})
  let _ = Repo.insert(pool, "profiles", %{"author_id" => "ada", "bio" => "mathematician"})
  show_int("execute_raw",
    Repo.execute_raw(pool, "UPDATE articles SET views = views + 1 WHERE author_id = $1", ["ada"]))
  failed("execute_raw_bad", Repo.execute_raw(pool, "UPDATE nowhere SET x = 1", []))
  show_rows("query_raw",
    Repo.query_raw(pool, "SELECT title FROM articles WHERE views > $1 ORDER BY title", ["15"]),
    "title")
  failed("query_raw_bad", Repo.query_raw(pool, "SELECT * FROM nowhere", []))
end

fn reads(pool :: PoolHandle) do
  let writers = Query.from("writers")
  show_rows("all",
    Repo.all(pool,
      writers
        |> Query.order_by(:handle, :asc)),
    "handle")
  show_rows("where_op",
    Repo.all(pool,
      writers
        |> Query.where_op(:score, :gte, "5")
        |> Query.order_by(:handle, :desc)),
    "handle")
  show_rows("where_in",
    Repo.all(pool,
      writers
        |> Query.where_in(:handle, ["ada", "cy"])
        |> Query.order_by(:handle, :asc)),
    "handle")
  show_rows("where_not_in",
    Repo.all(pool,
      writers
        |> Query.where_not_in(:handle, ["ada", "cy"])
        |> Query.order_by(:handle, :asc)),
    "handle")
  show_rows("where_between",
    Repo.all(pool,
      writers
        |> Query.where_between(:score, "2", "12")
        |> Query.order_by(:handle, :asc)),
    "handle")
  show_rows("where_null",
    Repo.all(pool,
      writers
        |> Query.where_null(:nickname)
        |> Query.order_by(:handle, :asc)),
    "handle")
  show_rows("where_not_null",
    Repo.all(pool,
      writers
        |> Query.where_not_null(:nickname)
        |> Query.order_by(:handle, :asc)),
    "handle")
  show_rows("where_or",
    Repo.all(pool,
      writers
        |> Query.where_or([:handle, :name], ["ada", "BOB"])
        |> Query.order_by(:handle, :asc)),
    "handle")
  show_rows("where_expr",
    Repo.all(pool,
      writers
        |> Query.where_expr(Expr.gt(Expr.column("score"), Expr.value("4")))
        |> Query.order_by(:handle, :asc)),
    "handle")
  show_rows("where_raw",
    Repo.all(pool,
      writers
        |> Query.where_raw("length(name) = ?", ["2"])
        |> Query.order_by(:handle, :asc)),
    "handle")
  show_rows("select_limit_offset",
    Repo.all(pool,
      writers
        |> Query.select(["handle"])
        |> Query.order_by(:handle, :asc)
        |> Query.limit(2)
        |> Query.offset(1)),
    "handle")
  show_rows("select_exprs",
    Repo.all(pool,
      writers
        |> Query.select_exprs([
          Expr.label(Expr.coalesce([Expr.column("nickname"), Expr.value("-")]), "shown"),
          Expr.label(Expr.case([Expr.gt(Expr.column("score"), Expr.value("5"))],
              [Expr.value("high")],
              Expr.value("low")),
            "band")
        ])
        |> Query.order_by(:handle, :asc)),
    "shown")
  show_rows("select_raw",
    Repo.all(pool,
      writers
        |> Query.select_raw(["upper(handle) AS big"])
        |> Query.order_by_raw("big DESC")),
    "big")
  show_rows("join",
    Repo.all(pool,
      Query.from("articles")
        |> Query.join(:inner, "writers", "writers.handle = articles.author_id")
        |> Query.select_raw(["articles.title AS title"])
        |> Query.where_raw("writers.name = ?", ["Ada"])
        |> Query.order_by_raw("articles.title")),
    "title")
  show_rows("join_as",
    Repo.all(pool,
      Query.from("articles")
        |> Query.join_as(:left, "writers", "w", "w.handle = articles.author_id")
        |> Query.select_raw(["w.name AS name"])
        |> Query.order_by_raw("articles.id")),
    "name")
  show_rows("group_having",
    Repo.all(pool,
      Query.from("articles")
        |> Query.select_raw(["author_id", "count(*)::text AS n"])
        |> Query.group_by(:author_id)
        |> Query.having("count(*) >", "1")),
    "author_id")
  show_rows("group_by_raw",
    Repo.all(pool,
      Query.from("articles")
        |> Query.select_raw(["lower(author_id) AS who"])
        |> Query.group_by_raw("lower(author_id)")
        |> Query.order_by_raw("who")),
    "who")
  show_rows("aggregates",
    Repo.all(pool,
      Query.from("articles")
        |> Query.select_count()
        |> Query.select_count_field(:title)
        |> Query.select_sum(:views)
        |> Query.select_min(:views)
        |> Query.select_max(:views)),
    "count")
  show_rows("where_sub",
    Repo.all(pool,
      writers
        |> Query.where_sub(:handle,
          Query.from("articles")
            |> Query.select(["author_id"])
            |> Query.where_op(:views, :gt, "15"))
        |> Query.order_by(:handle, :asc)),
    "handle")
  show_rows("fragment",
    Repo.all(pool,
      writers
        |> Query.fragment("ORDER BY score DESC LIMIT ?", ["1"])),
    "handle")
  failed("all_bad", Repo.all(pool, Query.from("nowhere")))
  show("one",
    Repo.one(pool,
      writers
        |> Query.where(:handle, "cy")),
    "name")
  failed("one_none",
    Repo.one(pool,
      writers
        |> Query.where(:handle, "zed")))
  failed("one_bad", Repo.one(pool, Query.from("nowhere")))
  show("get", Repo.get(pool, "articles", "2"), "title")
  failed("get_none", Repo.get(pool, "articles", "99"))
  failed("get_bad", Repo.get(pool, "nowhere", "1"))
  show("get_by", Repo.get_by(pool, "writers", "name", "Cy"), "handle")
  failed("get_by_none", Repo.get_by(pool, "writers", "name", "Zed"))
  failed("get_by_bad", Repo.get_by(pool, "nowhere", "name", "Zed"))
  show_int("count",
    Repo.count(pool,
      writers
        |> Query.where_op(:score, :lt, "8")))
  failed("count_bad", Repo.count(pool, Query.from("nowhere")))
  show_bool("exists",
    Repo.exists(pool,
      writers
        |> Query.where(:handle, "ada")))
  show_bool("exists_not",
    Repo.exists(pool,
      writers
        |> Query.where(:handle, "zed")))
  failed("exists_bad", Repo.exists(pool, Query.from("nowhere")))
end

# Every expression helper, in one row of `ada`, and an upsert whose update
# is qualified through each kind of expression.
fn expressions(pool :: PoolHandle) do
  show_int("pgcrypto", Pg.create_extension(pool, "pgcrypto"))
  show_fields("exprs",
    Repo.all(pool,
      Query.from("writers")
        |> Query.where_expr(Expr.neq(Expr.column("handle"), Expr.value("nobody")))
        |> Query.where_expr(Expr.lt(Expr.column("score"), Expr.value("1000")))
        |> Query.where_expr(Expr.lte(Expr.column("score"), Expr.value("100")))
        |> Query.where_expr(Expr.gte(Expr.column("score"), Expr.value("0")))
        |> Query.where_expr(Expr.eq(Expr.column("handle"), Expr.value("ada")))
        |> Query.select_exprs([
          Expr.label(Expr.sub(Expr.column("score"), Expr.value("1")), "less"),
          Expr.label(Expr.div(Pg.int(Expr.value("9")), Expr.value("3")), "third"),
          Expr.label(Pg.text(Pg.jsonb(Expr.value("{\"a\": 1}"))), "json"),
          Expr.label(Pg.jsonb_contains(Pg.jsonb(Expr.value("{\"a\": 1, \"b\": 2}")),
              Pg.jsonb(Expr.value("{\"a\": 1}"))),
            "contains"),
          Expr.label(Pg.uuid(Expr.value("00000000-0000-0000-0000-000000000001")), "id"),
          Expr.label(Pg.cast(Pg.timestamptz(Expr.value("2026-01-02T03:04:05Z")), "date"), "day"),
          Expr.label(Pg.tsvector_matches(Pg.to_tsvector("english", Expr.column("name")),
              Pg.plainto_tsquery("english", Expr.value("ada"))),
            "found"),
          Expr.label(Expr.gt(Pg.ts_rank(Pg.to_tsvector("english", Expr.value("mesh language")),
                Pg.plainto_tsquery("english", Expr.value("mesh"))),
              Expr.value("0")),
            "ranked"),
          Expr.label(Expr.call("length", [Pg.gen_salt("bf", 4)]), "salt"),
          Expr.label(Pg.crypt(Expr.value("pw"), Expr.value("$1$abcdefgh$")), "hash"),
          Expr.label(Expr.coalesce([Expr.null(), Expr.value("fallback")]), "value")
        ])),
    ["less", "third", "json", "contains", "id", "day", "found", "ranked", "salt", "hash", "value"])
  show("upsert_qualified",
    Repo.insert_or_update_expr(pool,
      "writers",
      %{"handle" => "ada", "name" => "Ada", "score" => "50", "nickname" => "A"},
      ["handle"],
      %{
        "nickname" => Expr.coalesce([Expr.column("nickname"), Expr.excluded("nickname")]),
        "score" => Expr.case([Expr.gt(Expr.excluded("score"), Expr.column("score"))],
          [Pg.cast(Expr.excluded("score"), "int")],
          Expr.column("score"))
      }),
    "score")
  failed("upsert_labelled",
    Repo.insert_or_update_expr(pool,
      "writers",
      %{"handle" => "ada", "name" => "Ada"},
      ["handle"],
      %{"score" => Expr.label(Expr.column("score"), "s")}))
  show_rows("select_star",
    Repo.all(pool,
      Query.from("writers")
        |> Query.select(["writers.*"])
        |> Query.where(:handle, "ada")),
    "nickname")
  case Pool.query_as(pool, "SELECT handle, score FROM writers ORDER BY handle", [], decode_score) do
    Ok(rows) -> println("query_as:" <> String.join(List.map(rows,
        fn(row) do
          case row do
            Ok(text) -> text
            Err(error) -> "error:" <> error
          end
        end),
      ","))
    Err(error) -> println("query_as:error:" <> error)
  end
  failed("query_as_bad", Pool.query_as(pool, "SELECT * FROM no_such_table", [], decode_score))
end

fn decode_score(row :: Map<String, String>) -> Result<String, String> do
  if Map.get(row, "handle") == "bob" do
    Err("bob has no decodable score")
  else
    Ok(Map.get(row, "handle") <> "=" <> Map.get(row, "score"))
  end
end

fn preloads(pool :: PoolHandle) do
  let meta = List.concat(List.concat(Author.__relationship_meta__(), Post.__relationship_meta__()),
    Comment.__relationship_meta__())
  let authors = Repo.all(pool,
    Query.from("writers")
      |> Query.where_in(:handle, ["ada", "bob", "cy"])
      |> Query.order_by(:handle, :asc))
  case authors do
    Ok(rows) -> do
      show_rows("preload_posts",
        Repo.preload(pool, rows, ["posts", "posts.comments", "profile"], meta),
        "posts")
      show_rows("preload_profile", Repo.preload(pool, rows, ["profile"], meta), "profile")
      failed("preload_unknown", Repo.preload(pool, rows, ["friends"], meta))
      failed("preload_unknown_nested", Repo.preload(pool, rows, ["friends.posts"], meta))
    end
    Err(error) -> println("preload:error:" <> error)
  end
  case Repo.all(pool,
    Query.from("articles")
      |> Query.order_by(:id, :asc)) do
    Ok(rows) -> show_rows("preload_author",
      Repo.preload(pool, rows, ["author", "author.profile"], meta),
      "author")
    Err(error) -> println("preload_author:error:" <> error)
  end
  show_rows("preload_empty", Repo.preload(pool, [], ["posts"], meta), "posts")
end

fn deletes(pool :: PoolHandle) do
  show("delete", Repo.delete(pool, "comments", "3"), "body")
  failed("delete_none", Repo.delete(pool, "comments", "99"))
  show_int("delete_where",
    Repo.delete_where(pool,
      "comments",
      Query.from("comments")
        |> Query.where(:body, "nice")))
  failed("delete_where_bad", Repo.delete_where(pool, "nowhere", Query.from("nowhere")))
  failed("delete_where_returning_unfiltered",
    Repo.delete_where_returning(pool, "comments", Query.from("comments")))
  show_rows("delete_where_returning",
    Repo.delete_where_returning(pool,
      "comments",
      Query.from("comments")
        |> Query.where_not_null(:body)),
    "body")
  failed("delete_where_returning_bad",
    Repo.delete_where_returning(pool, "nowhere", Query.from("nowhere")))
end

fn changesets(pool :: PoolHandle) do
  let valid = Changeset.cast(%{},
    %{"handle" => "fay", "name" => "Fay", "ignored" => "x"},
    [:handle, :name])
    |> Changeset.validate_required([:handle, :name])
  case Repo.insert_changeset(pool, "writers", valid) do
    Ok(row) -> println("insert_changeset:" <> Map.get(row, "name"))
    Err(cs) -> println("insert_changeset:error:" <> Json.encode(Changeset.errors(cs)))
  end
  let invalid = Changeset.cast(%{}, %{"handle" => "gus"}, [:handle, :name])
    |> Changeset.validate_required([:handle, :name])
  case Repo.insert_changeset(pool, "writers", invalid) do
    Ok(_) -> println("insert_changeset_invalid:unexpected-ok")
    Err(cs) -> println("insert_changeset_invalid:" <> Changeset.get_error(cs, :name))
  end
  let duplicate = Changeset.cast(%{}, %{"handle" => "hal", "name" => "Fay"}, [:handle, :name])
  case Repo.insert_changeset(pool, "writers", duplicate) do
    Ok(_) -> println("insert_changeset_duplicate:unexpected-ok")
    Err(cs) -> println("insert_changeset_duplicate:" <> Changeset.get_error(cs, :name))
  end
  let rename = Changeset.cast(%{}, %{"name" => "Faye"}, [:name])
  case Repo.update_changeset(pool, "writers", "fay", rename) do
    Ok(row) -> println("update_changeset:" <> Map.get(row, "name"))
    Err(cs) -> println("update_changeset:error:" <> Json.encode(Changeset.errors(cs)))
  end
  let taken = Changeset.cast(%{}, %{"name" => "Ada"}, [:name])
  case Repo.update_changeset(pool, "writers", "fay", taken) do
    Ok(_) -> println("update_changeset_duplicate:unexpected-ok")
    Err(cs) -> println("update_changeset_duplicate:" <> Changeset.get_error(cs, :name))
  end
  let blank = Changeset.cast(%{}, %{}, [:name])
    |> Changeset.validate_required([:name])
  case Repo.update_changeset(pool, "writers", "fay", blank) do
    Ok(_) -> println("update_changeset_invalid:unexpected-ok")
    Err(cs) -> println("update_changeset_invalid:" <> Changeset.get_error(cs, :name))
  end
end

fn add_article(conn :: borrow PgConn) -> String!String do
  let _ = Pg.execute(conn,
    "INSERT INTO articles (author_id, title) VALUES ($1, $2)",
    ["cy", "Kept"])?
  Ok("committed")
end

fn add_then_fail(conn :: borrow PgConn) -> String!String do
  let _ = Pg.execute(conn,
    "INSERT INTO articles (author_id, title) VALUES ($1, $2)",
    ["cy", "Dropped"])?
  Err("rolled back")
end

fn transactions(pool :: PoolHandle) do
  case Repo.transaction(pool, add_article) do
    Ok(message) -> println("transaction:" <> message)
    Err(error) -> println("transaction:error:" <> error)
  end
  case Repo.transaction(pool, add_then_fail) do
    Ok(_) -> println("transaction_rollback:unexpected-ok")
    Err(error) -> println("transaction_rollback:" <> error)
  end
  show_int("transaction_titles",
    Repo.count(pool,
      Query.from("articles")
        |> Query.where_in(:title, ["Kept", "Dropped"])))
end

fn run() -> Int!String do
  let url = Env.get("MESH_TEST_DATABASE_URL",
    "postgres://mesh_test:mesh_test@localhost:5432/mesh_test?sslmode=disable")
  let pool = Pool.open(url, 1, 1, 5000)?
  let _ = setup(pool)?
  writes(pool)
  reads(pool)
  expressions(pool)
  preloads(pool)
  changesets(pool)
  transactions(pool)
  deletes(pool)
  let _ = Pool.execute(pool, "DROP SCHEMA mesh_repo_e2e CASCADE", [])?
  Pool.close(pool)
  Ok(0)
end

fn main() do
  case run() do
    Ok(_) -> println("done")
    Err(error) -> println("error:" <> error)
  end
end
