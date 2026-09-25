# leading
import Foo # after import
from Bar import ( # open
  a, # a
  b # b
) # close
from Baz import c, d # tail

@cluster(2) # decorator
pub fn clustered() -> Int do # sig
  1 # body
end # end

fn guarded(x) when x > 0 = x # expr body
fn guarded(x) = 0

fn f(a, # a
     b) do # after do
  let x = # after eq
    1
  if a do # if
    x # then
  else # else
    b
  end # end if
  if a do
    1
  else if b do # elif
    2
  else
    3
  end
  while false do # while
    break # brk
  end
  for i in [1, 2] do # for
    i
  end
  for {k, v} in %{"a" => 1} do # destructure
    k
  end
  case x do # case
    1 -> 2 # arm
    _ -> # arrow
      3
  end
  let y = -x # unary
  let z = not true # not
  let w = x + # plus
    1
  let p = x
    |> g() # pipe
  let s = S { a: 1, # field
    b: 2 }
  let u = %{s | a: 3} # update
  let m = %{"a" => 1, # entry
    "b" => 2}
  let j = json { a: 1, # json
    b: 2 }
  let l = [1, # one
    2]
  let t = (1, # tuple
    2)
  let cl = fn x -> # closure
    x
  end
  let cl2 = fn 0 -> 1 | n -> n end # clauses
  h(x) do |v| # trailing
    v
  end
  return x # ret
end

struct S do # struct
  a :: Int # a
  b :: Int
end deriving(Eq) # deriving

type Color do
  Red; Green # variants on one line
end deriving(Eq, # first trait
  Show)

type Shape do # sum
  Circle(Float) # circle
  Dot
end

type Alias = Int # alias

interface Named do # iface
  fn name(self) -> String # sig
end

impl Named for S do # impl
  fn name(self) -> String do
    "s" # s
  end
end

actor worker() do # actor
  receive do # recv
    m -> m # arm
    after 10 -> 0 # after
  end
  terminate do # term
    1
  end
end

service Store do # svc
  fn init() -> Int do
    0
  end
  call Get() :: Int do |s| # call
    (s, s)
  end
  cast Put(v :: Int) do |s| # cast
    v
  end
end

supervisor Sup do # sup
  strategy: one_for_one # strat
  max_restarts: 3
  max_seconds: 5
  child w do # child
    start: fn -> spawn(worker) end # start
    restart: permanent # restart
    shutdown: 5000
  end
end

module Inner do # mod
  fn g() = 1
end

@native("mesh_math_add") # native
pub fn native_add(left :: Int, right :: Int) -> Int

@export("mesh_echo") # export
pub fn echo(request :: Bytes) -> Bytes!String do
  Ok(request) # ok
end

type Pair<A, B> = (A, B) # generic alias

type Outcome<T> do # generic sum
  Pending
  Complete(value :: T) # named
end

interface Container do
  type Item # assoc
  fn first(self) -> Self.Item
end

impl Container for S do
  type Item = Int # binding
  fn first(self) -> Int do
    self.a
  end
end

fn more(xs, pid, m) do
  link(pid) # link
  let me = self() # self
  let r = parse(xs)? # try
  let slot = 10 |2> sub(1) # slot pipe
  request("/x", method: "POST", retries: 3) # keyword args
  let text = """
    hello #{r}
    """ # heredoc
  let rx = ~r/[a-z]+/i # regex
  let at = :ready # atom
  let keyed = %{:a => 1} # atom map
  let item = xs[0] # index
  for x in xs when x > 1 do # filter
    x
  end
  receive do
    n when n > 0 -> n # guard
  end
  case m do
    [] -> 0 # empty
    h :: t -> h # cons
    Some(1 | 2) -> 1 # or
    (a, b) as pair -> a # as
    -1 -> 2 # negative
  end
  let e = %{"a" => # value
    1}
  let w = %{s | # field
    a: 3}
  for {k, # key
    v} in m do
    k
  end
  case p do
    P { x, # pattern field
      y } -> x
  end
  return # bare return
end

fn cons_param(_ :: rest) = rest # cons param

module Blocks #= module =# do
  fn a() = 1 #= after =# + 2
end

struct Boxed #= struct =# do
  a :: Int
end

@cluster(#= count =# 3)
fn clustered() do
  g(1 #= one =#, 2)
end

impl Show for #= impl =# Boxed do
  fn show(self) -> String do
    "boxed"
  end
end

from Bar #= path =# import x

supervisor Sup do
  child w #= name =# do
    start: fn -> 1 end
  end
end

fn block_comments_in_rare_places(xs) do
  let g = fn 0 -> 1 | n #= clause =# -> n end
  let h = fn a #= first =#, b -> a end
  g(1, name: #= kw =# 2)
  return #= value =# 1
end
