case System.argv() do
  ["--" | args] -> JobsParityOban.run(args)
  args -> JobsParityOban.run(args)
end
