use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const PROOF_HTTP_READ_TIMEOUT: Duration = Duration::from_secs(15);

pub type BurstResult = (Result<HttpResponse, String>, u64);

#[derive(serde::Serialize, serde::Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpResponse {
    #[allow(dead_code)]
    pub fn header(&self, wanted: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.as_str())
    }
}

pub fn http_request(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    extra_headers: &[(&str, &str)],
) -> Result<HttpResponse, String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|error| format!("proof_http_connect_failed:{address}:{error}"))?;
    stream
        .set_read_timeout(Some(PROOF_HTTP_READ_TIMEOUT))
        .map_err(|error| format!("proof_http_timeout_failed:{error}"))?;
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("proof_http_write_failed:{error}"))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("proof_http_read_failed:{error}"))?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .unwrap_or((response.as_str(), ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse().ok())
        .ok_or_else(|| "proof_http_status_invalid".to_string())?;
    let headers = lines
        .take_while(|line| !line.is_empty() && *line != "\r")
        .filter_map(|line| line.trim_end_matches('\r').split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    Ok(HttpResponse {
        status,
        headers,
        body: body.to_string(),
    })
}
