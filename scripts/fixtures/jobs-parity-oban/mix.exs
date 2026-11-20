defmodule JobsParityOban.MixProject do
  use Mix.Project

  def project do
    [
      app: :jobs_parity_oban,
      version: "0.1.0",
      elixir: "~> 1.18",
      deps: [
        {:oban, "== 2.24.2"},
        {:postgrex, "~> 0.22"},
        {:jason, "~> 1.4"}
      ]
    ]
  end
end
