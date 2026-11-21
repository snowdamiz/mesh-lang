fn drain(pool :: PoolHandle, config :: String, _options :: String) -> String!String do
  let runner = Jobs.start(pool, config, handle)?
  Jobs.join(runner)
end
