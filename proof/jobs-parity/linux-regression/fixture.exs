Application.load(:oban)
args = case System.argv() do
  ["--" | args] -> args
  args -> args
end

if args == ["session"] do
  Code.require_file("/proof/oban-session.ex")
end

JobsParityOban.run(args)
