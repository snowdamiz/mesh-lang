defmodule PerformanceElixir.HTTP do
  @behaviour Plug

  import Plug.Conn

  alias PerformanceElixir.Score

  @impl true
  def init(opts), do: opts

  @impl true
  def call(%{method: "GET", request_path: "/health"} = conn, _opts) do
    json(conn, 200, %{ok: true})
  end

  def call(%{method: "POST", request_path: "/score"} = conn, _opts) do
    case read_body(conn, length: 1_048_576) do
      {:ok, body, conn} -> respond_with_score(conn, body)
      {:more, _body, conn} -> json(conn, 400, %{error: "invalid body"})
      {:error, _reason} -> json(conn, 400, %{error: "invalid body"})
    end
  end

  def call(conn, _opts), do: json(conn, 404, %{error: "not found"})

  defp respond_with_score(conn, body) do
    with {:ok, record} <- JSON.decode(body),
         {:ok, {id, _values} = record} <- Score.record(record) do
      json(conn, 200, %{id: id, score: Score.compute(record)})
    else
      _ -> json(conn, 400, %{error: "invalid record"})
    end
  end

  defp json(conn, status, data) do
    conn
    |> put_resp_header("content-type", "application/json")
    |> send_resp(status, JSON.encode_to_iodata!(data))
  end
end
