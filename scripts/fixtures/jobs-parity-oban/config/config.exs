import Config

config :jobs_parity_oban, ecto_repos: [JobsParityOban.Repo]
config :logger, :default_handler, config: [type: :standard_error]
