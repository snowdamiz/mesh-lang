use mesh_typeck::check;

#[test]
fn scheduler_aware_http_configuration_and_metrics_type_check() {
    let parsed = mesh_parser::parse(
        r##"
fn main() do
  let request = Http.build(:get, "https://example.com/data")
    |> Http.timeout(30_000)
    |> Http.stage_timeout(:resolve, 2_000)
    |> Http.stage_timeout(:connect, 5_000)
    |> Http.stage_timeout(:send, 5_000)
    |> Http.stage_timeout(:first_byte, 10_000)
    |> Http.stage_timeout(:body, 10_000)
    |> Http.max_redirects(0)
    |> Http.max_response_bytes(1_048_576)
  let class = Http.retry_class(:post, "TIMEOUT_CONNECT: timed out")
  let metrics = Http.metrics()
  println("#{class}:#{metrics.requests}:#{metrics.in_flight}:#{metrics.dns_micros}:#{metrics.connect_micros}:#{metrics.tls_micros}:#{metrics.first_byte_micros}:#{metrics.total_micros}:#{metrics.response_bytes}")
  case Http.send(request) do
    Ok(response) -> println("#{response.status}")
    Err(error) -> println(error)
  end
end
"##,
    );
    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    let result = check(&parsed);
    assert!(
        result.errors.is_empty(),
        "unexpected type errors: {:#?}",
        result.errors
    );
}
