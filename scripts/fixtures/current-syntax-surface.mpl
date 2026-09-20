##! Current syntax surface fixture.
## Compiler-derived highlighting probes.
# This fixture intentionally ends with invalid-token probes.
#= outer block #= nested block =# still outer block =#

@native("mesh_u128_add")
pub fn native_add(left :: U128, right :: U128) -> U128

@ native ("mesh_u128_identity")
pub fn native_identity(value :: U128) -> U128

@ cluster ( 3 )
pub fn spaced_cluster() -> Int do
  3
end

from Solana.Read import pubkey
import App.Models.User

pub type Δelta = Int

pub interface HealthProbe do
  fn ping -> Int
  fn default_ping -> Int do
    1
  end
  fn default_with_multiline_closure -> Int do
    let callbacks = [
      fn value when predicate("a=b") and Regex.matches(~r/a=b/i, value) # guard=a=b
        -> 1
      end,
    ]
    1
  end
  fn after_multiline_default
  fn reset
end

let interface_seed = 1; interface SemicolonHealthProbe do
  fn semicolon_reset
end

pub #= visibility trivia =# interface #= interface-name trivia =# CommentTriviaHealthProbe #= interface-do trivia =# do
  fn #= interface-method trivia =# commented_ping
end

pub struct User do
  table "users"
  primary_key :uuid
  timestamps true
  timestamps false

  id :: U64
  balance :: I128
  payload :: Bytes
  matcher :: Regex

  belongs_to :account, Account
  has_one :profile, Profile
  has_many :posts, Post
end deriving(Schema, Row, Json)

pub struct CommentTriviaUser do
  table #= table trivia =# "comment_users"
  primary_key #= primary-key trivia =# :uuid
  timestamps #= timestamps trivia =# true
  belongs_to #= relationship trivia =# :account #= relationship-comma trivia =# , Account
end #= deriving trivia =# deriving #= deriving-call trivia =# (Schema)

pub #= visibility trivia =# supervisor #= supervisor-name trivia =# CommentTriviaSupervisor #= supervisor-do trivia =# do
  strategy #= strategy-colon trivia =# : #= strategy-value trivia =# one_for_one
  max_restarts #= max-restarts trivia =# : 2

  child #= child-name trivia =# CommentTriviaWorker #= child-do trivia =# do
    start #= start-colon trivia =# : fn do
      fn #= declaration-name trivia =# helper = 1
      helper
    end
    restart #= restart-colon trivia =# : #= restart-value trivia =# permanent
    shutdown #= shutdown-colon trivia =# : #= shutdown-value trivia =# brutal_kill
  end
end

pub supervisor AppSupervisor do
  strategy: one_for_all
  strategy: one_for_one
  strategy: rest_for_one
  strategy: simple_one_for_one
  max_restarts: 5
  max_seconds: 10

  child WorkerPool do
    start: fn -> spawn(Worker) end
    restart: permanent
    restart: transient
    restart: temporary
    shutdown: brutal_kill
    shutdown: 5000
  end

  child NestedWorker do
    start: wrap(
      fn value when value == """
        a=b
      """
        do
        configure(
          strategy: one_for_one,
          restart: transient,
          shutdown: brutal_kill,
        )
        spawn(Worker)
      end
    )
    restart: permanent # split-do-restart
    shutdown: 5000
  end

  child WrappedWorker do
    start: wrap(fn -> wrap(fn -> spawn(Worker) end) end)
    restart: permanent # wrapped-start-restart
    shutdown: 5000
  end

  child MultilineWrappedWorker do
    start: wrap(
      fn value when predicate("""
        a=b
      """)
        -> wrap(
          fn
            -> spawn(Worker)
          end
        )
      end
    )
    restart: temporary # multiline-wrapped-start-restart
    shutdown: 5000
  end

  child WrappedDoWorker do
    start: wrap(fn do wrap(fn do spawn(Worker) end) end)
    restart: transient # wrapped-do-restart
    shutdown: 5000
  end

  child MarkerAwareArrowWorker do
    start: wrap(fn value when predicate("->", ~r/do->/) -> spawn(Worker) end)
    restart: permanent # marker-aware-arrow-restart
    shutdown: 5000
  end

  child MarkerAwareDoWorker do
    start: wrap(fn value when value == "do" do spawn(Worker) end)
    restart: transient # marker-aware-do-restart
    shutdown: 5000
  end

  child MultilineHeaderWorker do
    start: wrap(
      fn value when predicate(
        value,
        "->",
      )
        # marker follows -> do
        #= outer marker -> #= nested do =# =#
        -> spawn(Worker)
      end
    )
    restart: temporary # multiline-header-restart
    shutdown: 5000
  end

  child LexicallyMaskedHeaderWorker do
    start: wrap(
      fn value when predicate(
        "-> do end",
        ~r/->doend/,
        """
          -> do end
        """,
      )
        # marker text -> do end
        #= outer -> do end #= nested -> do end =# =#
        -> spawn(Worker)
      end
    )
    restart: permanent # lexically-masked-header-restart
    shutdown: 5000
  end

  child TrailingDoClosureWorker do
    start: fn -> Builder.make() do
      spawn(Worker)
    end end
    restart: transient # trailing-do-closure-restart
    shutdown: 5000
  end

  child CaseArmDoClosureWorker do
    start: fn state -> case state do
      _ -> do
        spawn(Worker)
      end
    end end
    restart: temporary # case-arm-do-closure-restart
    shutdown: 5000
  end

  child EmptyClosureWorker do
    start: choose(fn -> end, fn do end)
    restart: permanent # empty-closure-restart
    shutdown: 5000
  end

  child BodyLexicalEndWorker do
    start: fn do
      let markers = [
        "end",
        ~r/end/,
        """
          end
        """,
      ]
      # line-comment end marker
      #= outer end #= nested end =# still outer =#
      spawn(Worker)
    end
    restart: transient # body-lexical-end-restart
    shutdown: 5000
  end

  child ElseIfClosureWorker do
    start: fn value -> if value > 0 do
      spawn(Worker)
    else #= branch #= nested =# trivia =# if value == 0 do
      spawn(Worker)
    else
      spawn(Worker)
    end end
    restart: temporary # else-if-closure-restart
    shutdown: 5000
  end

  child MultilineCommentElseIfClosureWorker do
    start: fn value -> if value > 0 do
      spawn(Worker)
    else #=
      branch trivia
    =# if value == 0 do
      spawn(Worker)
    else
      spawn(Worker)
    end end
    restart: permanent # multiline-comment-else-if-closure-restart
    shutdown: 5000
  end

  child NestedGuardClosureWorker do
    start: fn value when predicate(fn nested -> nested end) -> spawn(Worker) end
    restart: permanent # nested-guard-closure-restart
    shutdown: 5000
  end

  child TypedClosureWorker do
    start: fn callback :: Fun(Int) -> Pid do
      callback(1)
    end
    restart: transient # typed-closure-restart
    shutdown: 5000
  end

  child GuardBlockClosureWorker do
    start: fn value when if value > 0 do true else false end -> spawn(Worker) end
    restart: temporary # guard-block-closure-restart
    shutdown: 5000
  end

  child NestedDeclarationWorker do
    start: fn do
      fn helper = 1
      fn helper_with_param(value) = value
      helper_with_param(helper)
    end
    restart: permanent # nested-declaration-restart
    shutdown: 5000
  end
end

let supervisor_seed = 1; supervisor SemicolonSupervisor do
  max_restarts: 13
end

pub supervisor InlineSupervisor do
  strategy: one_for_one; max_restarts: 3; child InlineWorker do
    start: fn -> spawn(Worker) end; restart: transient; shutdown: brutal_kill
  end; max_seconds: 7
end

service ProbeService do
  call Get :: Int do |state|
    (state, state)
  end

  cast Reset do |state|
    state
  end
end

pub fn heartbeat -> Bool do
  true
end

pub def heartbeat_alias -> Bool do
  true
end

fn private_heartbeat -> Bool do
  true
end

def private_heartbeat_alias -> Bool do
  true
end

fn private_pair -> (Int, String) do
  (1, "one")
end

fn guarded_tick when ready = ready

fn typed_expression -> Int = 42

fn guarded_block when ready do
  ready
end

pub fn load_user(id :: U64) -> User!String do
  let query = Query.from(User)
  let scoped = Query.where(query, Expr.eq(Expr.column("id"), Expr.value(id)))
  let changes = Changeset.cast(%{}, %{name => String})
  let migration = Migration.create_table("users")
  let user = Repo.one(scoped)?
  let updated = %{user | name: "Ada"}
  let matcher = ~r/users\/[0-9]+/ims
  let multiline_matcher = ~r/^first$
  ^second$/ms
  let after_multiline_matcher = 1
  let orderings = [Less, Equal, Greater]
  let constructors = [Some(1), None, Ok(1), Err("error")]
  let iterator_types = [ListIterator, MapIterator, SetIterator, RangeIterator]
  let numeric_literals = [42, 0xFF, 0b1010, 0o77, 3.14, 1.0e10]
  let transform = fn value -> value end
  let guarded_transform = fn value when value > 0 -> value end
  let transforms = [
    fn value when value > 0 -> value end,
  ]
  let multiline_closures = [
    fn closure_value -> Int
    end,
    fn equal_value when equal_value == 0 -> equal_value end,
    fn guarded_do_value when guarded_do_value > 0 do
      guarded_do_value
    end,
    fn Some(constructor_value) -> constructor_value end,
    fn option_value -> Some("a=b") end,
    fn builder_value -> Builder.make() do
      builder_value
    end end,
  ]
  let configured = configure(strategy: one_for_one, restart: transient, shutdown: brutal_kill)
  let configured_multiline = configure(
    strategy: one_for_one,
    restart: transient,
    shutdown: brutal_kill,
  )
  let derived = deriving(value)
  let selected = case user do
    Ok(value) | Some(value) when value > 0 -> value
    None as missing -> missing
    _ -> updated
  end
  let κόσμος = pubkey()
  let Ⅳalue = 4
  let 四季 = 4
  let inverted = !false
  println("#{κόσμος}")
  selected
end

fn contextual_names(native, table, strategy, deriving, from, as) do
  native(table, strategy, deriving, from, as)
end

fn unicode_boundary_names(letπ, Inté, Someδ, trueλ) do
  [letπ, Inté, Someδ, trueλ]
end

# Invalid-token and lexical-boundary probes: these must not receive the valid
# decorator/literal/operator scopes of the similar-looking supported forms.
@clusterπ
@nativeπ("symbol")
:Invalid
~r
left |0> right
left |1> right
left |2 right
