defmodule PerformanceElixir.MixProject do
  use Mix.Project

  def project do
    [
      app: :performance_elixir,
      version: "0.1.0",
      elixir: "~> 1.20",
      deps: [{:bandit, "~> 1.12"}, {:plug, "~> 1.19"}]
    ]
  end

  def application do
    [mod: {PerformanceElixir.Application, []}, extra_applications: [:logger]]
  end
end
