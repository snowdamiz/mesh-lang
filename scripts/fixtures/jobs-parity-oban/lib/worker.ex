defmodule JobsParityOban.Worker do
  use Oban.Worker, queue: :parity, max_attempts: 3

  alias JobsParityOban.Repo

  @impl Oban.Worker
  def backoff(_job), do: 1

  @impl Oban.Worker
  def perform(%Oban.Job{
        attempt: attempt,
        conf: %Oban.Config{prefix: namespace},
        args: %{"key" => key, "hold_ms" => hold_ms, "fail_first" => fail_first}
      }) do
    entered_at = DateTime.utc_now()

    Repo.checkout(fn ->
      record_attempt(namespace, key, attempt, entered_at)
      if not fail_first and hold_ms > 0, do: Process.sleep(hold_ms)
      write_effect(namespace, key)
    end)

    if fail_first and attempt == 1 do
      if hold_ms > 0, do: Process.sleep(hold_ms)
      {:error, :fixture_fail_first}
    else
      :ok
    end
  end

  defp record_attempt(namespace, key, attempt, entered_at) do
    Repo.query!(
      """
      INSERT INTO public.mesh_jobs_parity_attempts
        (engine, namespace, job_key, effect_key, attempt, entered_at, worker_pid)
      VALUES ('oban', $1, $2, $2, $3, $4, $5)
      """,
      [namespace, key, attempt, entered_at, String.to_integer(System.pid())]
    )
  end

  defp write_effect(namespace, key) do
    Repo.query!(
      """
      INSERT INTO public.mesh_jobs_parity_effects
        (engine, namespace, job_key, effect_key, effected_at)
      VALUES ('oban', $1, $2, $2, clock_timestamp())
      ON CONFLICT DO NOTHING
      """,
      [namespace, key]
    )
  end
end
