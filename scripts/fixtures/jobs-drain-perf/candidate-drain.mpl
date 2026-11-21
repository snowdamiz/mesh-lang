fn drain(pool :: PoolHandle, config :: String, options :: String) -> String!String do
  Jobs.drain(pool, config, handle, options)
end
