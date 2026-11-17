use std::convert::Infallible;

use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    header::{CONTENT_TYPE, HeaderValue},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use tokio::net::TcpListener;

use crate::{Result, record::Record};

#[derive(Serialize)]
struct ScoredRecord {
    id: i64,
    score: i64,
}

async fn handle(
    request: Request<Incoming>,
) -> std::result::Result<Response<Full<Bytes>>, Infallible> {
    let response = match (request.method(), request.uri().path()) {
        (&Method::GET, "/health") => json(StatusCode::OK, r#"{"ok":true}"#),
        (&Method::POST, "/score") => score(request.into_body()).await,
        _ => json(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#),
    };
    Ok(response)
}

async fn score(body: Incoming) -> Response<Full<Bytes>> {
    if let Ok(body) = Limited::new(body, 1 << 20).collect().await
        && let Ok(record) = serde_json::from_slice::<Record>(&body.to_bytes())
    {
        let result = ScoredRecord {
            id: record.id,
            score: record.score(),
        };
        let body = serde_json::to_vec(&result).expect("score contains only integers");
        return json(StatusCode::OK, body);
    }
    json(StatusCode::BAD_REQUEST, r#"{"error":"invalid record"}"#)
}

fn json(status: StatusCode, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body.into()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

pub(crate) async fn serve(port: u16) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(error) = http1::Builder::new()
                .keep_alive(false)
                .serve_connection(TokioIo::new(stream), service_fn(handle))
                .await
            {
                eprintln!("HTTP connection: {error}");
            }
        });
    }
}
