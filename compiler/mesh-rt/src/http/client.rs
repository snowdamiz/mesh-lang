//! Scheduler-aware, bounded HTTP client runtime.

use std::collections::HashMap;
use std::fmt;
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{
    Buffers, ConnectProxyConnector, ConnectionDetails, Connector, NextTimeout, RustlsConnector,
    TcpConnector, Transport,
};
use ureq::{Agent, Error as UreqError, RequestBuilder};
use url::Url;

use crate::actor::{cooperative_channel, cooperative_recv_timeout, CooperativeSender};
use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::collections::map::{mesh_map_new_typed, mesh_map_put};
use crate::gc::mesh_gc_alloc_actor;
use crate::io::alloc_result;
use crate::string::{mesh_string_new, MeshString};

const MAX_OPEN_HANDLES: usize = 4_096;
const MAX_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_REDIRECTS: u32 = 20;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const STREAM_CHUNK_BYTES: usize = 8 * 1024;

#[derive(Clone)]
struct RequestTimeouts {
    global: Duration,
    resolve: Duration,
    connect: Duration,
    send: Duration,
    first_byte: Duration,
    body: Duration,
}

impl Default for RequestTimeouts {
    fn default() -> Self {
        Self {
            global: Duration::from_secs(30),
            resolve: Duration::from_secs(5),
            connect: Duration::from_secs(10),
            send: Duration::from_secs(10),
            first_byte: Duration::from_secs(15),
            body: Duration::from_secs(30),
        }
    }
}

struct MeshRequestData {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    is_json: bool,
    query_params: Vec<(String, String)>,
    timeouts: RequestTimeouts,
    max_redirects: Option<u32>,
    max_response_bytes: usize,
    config_error: Option<String>,
}

impl MeshRequestData {
    fn new(method: &str, url: &str) -> Self {
        Self {
            method: method.to_ascii_lowercase(),
            url: url.to_string(),
            headers: Vec::new(),
            body: None,
            is_json: false,
            query_params: Vec::new(),
            timeouts: RequestTimeouts::default(),
            max_redirects: None,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            config_error: None,
        }
    }
}

#[derive(Debug)]
struct WorkerResponse {
    status: i64,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}

enum WorkerEvent {
    Complete(Result<WorkerResponse, String>),
    Cancelled,
}

struct ActiveRequest {
    cancel: Arc<AtomicBool>,
    sender: CooperativeSender<WorkerEvent>,
}

#[repr(C)]
pub struct MeshClientResponse {
    pub status: i64,
    pub body: *mut u8,
    pub headers: *mut u8,
    pub body_bytes: *mut u8,
}

#[repr(C)]
pub struct MeshHttpClientMetrics {
    pub requests: i64,
    pub in_flight: i64,
    pub dns_micros: i64,
    pub connect_micros: i64,
    pub tls_micros: i64,
    pub dns_failures: i64,
    pub connect_failures: i64,
    pub tls_failures: i64,
    pub timeouts: i64,
    pub first_byte_micros: i64,
    pub total_micros: i64,
    pub response_bytes: i64,
    pub cancellations: i64,
}

#[derive(Default)]
struct HttpMetrics {
    requests: AtomicU64,
    in_flight: AtomicU64,
    dns_micros: AtomicU64,
    connect_micros: AtomicU64,
    tls_micros: AtomicU64,
    dns_failures: AtomicU64,
    connect_failures: AtomicU64,
    tls_failures: AtomicU64,
    timeouts: AtomicU64,
    first_byte_micros: AtomicU64,
    total_micros: AtomicU64,
    response_bytes: AtomicU64,
    cancellations: AtomicU64,
}

struct TimedResolver(DefaultResolver);

impl fmt::Debug for TimedResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TimedResolver")
    }
}

impl Resolver for TimedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, UreqError> {
        let started = Instant::now();
        let result = self.0.resolve(uri, config, timeout);
        metrics()
            .dns_micros
            .fetch_add(duration_micros(started).max(1), Ordering::Relaxed);
        result
    }
}

struct TimedConnector<C>(C);

impl<C> fmt::Debug for TimedConnector<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TimedConnector")
    }
}

impl<In, C> Connector<In> for TimedConnector<C>
where
    In: Transport,
    C: Connector<In>,
{
    type Out = C::Out;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, UreqError> {
        let started = Instant::now();
        let result = self.0.connect(details, chained);
        metrics()
            .connect_micros
            .fetch_add(duration_micros(started).max(1), Ordering::Relaxed);
        result
    }
}

struct TimedTlsConnector<C>(C);

impl<C> fmt::Debug for TimedTlsConnector<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TimedTlsConnector")
    }
}

impl<In, C> Connector<In> for TimedTlsConnector<C>
where
    In: Transport,
    C: Connector<In>,
{
    type Out = TimedTlsTransport<C::Out>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, UreqError> {
        let timed = details.needs_tls();
        self.0.connect(details, chained).map(|transport| {
            transport.map(|inner| TimedTlsTransport {
                inner,
                timed,
                recorded: false,
            })
        })
    }
}

struct TimedTlsTransport<T> {
    inner: T,
    timed: bool,
    recorded: bool,
}

impl<T> fmt::Debug for TimedTlsTransport<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TimedTlsTransport")
    }
}

impl<T: Transport> Transport for TimedTlsTransport<T> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), UreqError> {
        if !self.timed || self.recorded {
            return self.inner.transmit_output(amount, timeout);
        }
        let started = Instant::now();
        let result = self.inner.transmit_output(amount, timeout);
        metrics()
            .tls_micros
            .fetch_add(duration_micros(started).max(1), Ordering::Relaxed);
        self.recorded = true;
        result
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, UreqError> {
        self.inner.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

fn http_agent() -> Agent {
    let connector =
        ().chain(ConnectProxyConnector::default())
            .chain(TimedConnector(TcpConnector::default()))
            .chain(TimedTlsConnector(RustlsConnector::default()));
    Agent::with_parts(
        ureq::config::Config::default(),
        connector,
        TimedResolver(DefaultResolver::default()),
    )
}

struct InFlight;

impl InFlight {
    fn begin() -> Self {
        metrics().requests.fetch_add(1, Ordering::Relaxed);
        metrics().in_flight.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        metrics().in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

static REQUESTS: OnceLock<Mutex<HashMap<u64, MeshRequestData>>> = OnceLock::new();
static ACTIVE_REQUESTS: OnceLock<Mutex<HashMap<u64, ActiveRequest>>> = OnceLock::new();
static CLIENTS: OnceLock<Mutex<HashMap<u64, Agent>>> = OnceLock::new();
static STREAMS: OnceLock<Mutex<HashMap<u64, Arc<AtomicBool>>>> = OnceLock::new();
static METRICS: OnceLock<HttpMetrics> = OnceLock::new();
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn requests() -> &'static Mutex<HashMap<u64, MeshRequestData>> {
    REQUESTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_requests() -> &'static Mutex<HashMap<u64, ActiveRequest>> {
    ACTIVE_REQUESTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn clients() -> &'static Mutex<HashMap<u64, Agent>> {
    CLIENTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn streams() -> &'static Mutex<HashMap<u64, Arc<AtomicBool>>> {
    STREAMS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn metrics() -> &'static HttpMetrics {
    METRICS.get_or_init(HttpMetrics::default)
}

fn next_handle() -> u64 {
    NEXT_HANDLE.fetch_add(1, Ordering::Relaxed)
}

fn update_request(handle: u64, update: impl FnOnce(&mut MeshRequestData)) -> u64 {
    let mut registry = requests().lock();
    let Some(request) = registry.get_mut(&handle) else {
        return 0;
    };
    update(request);
    handle
}

fn set_timeout(
    timeout: &mut Duration,
    config_error: &mut Option<String>,
    millis: i64,
    label: &str,
) {
    let Ok(millis) = u64::try_from(millis) else {
        *config_error = Some(format!("{label} timeout must be positive"));
        return;
    };
    let value = Duration::from_millis(millis);
    if value.is_zero() || value > MAX_TIMEOUT {
        *config_error = Some(format!(
            "{label} timeout must be between 1 and 120000 milliseconds"
        ));
    } else {
        *timeout = value;
    }
}

#[no_mangle]
pub extern "C" fn mesh_http_build(method: *const MeshString, url: *const MeshString) -> u64 {
    if method.is_null() || url.is_null() {
        return 0;
    }
    let mut registry = requests().lock();
    if registry.len() >= MAX_OPEN_HANDLES {
        return 0;
    }
    let method = unsafe { (*method).as_str() };
    let url = unsafe { (*url).as_str() };
    let handle = next_handle();
    registry.insert(handle, MeshRequestData::new(method, url));
    handle
}

#[no_mangle]
pub extern "C" fn mesh_http_header(
    handle: u64,
    key: *const MeshString,
    value: *const MeshString,
) -> u64 {
    if key.is_null() || value.is_null() {
        return 0;
    }
    let key = unsafe { (*key).as_str().to_string() };
    let value = unsafe { (*value).as_str().to_string() };
    update_request(handle, |request| request.headers.push((key, value)))
}

#[no_mangle]
pub extern "C" fn mesh_http_body(handle: u64, body: *const MeshString) -> u64 {
    if body.is_null() {
        return 0;
    }
    let body = unsafe { (*body).as_str().as_bytes().to_vec() };
    update_request(handle, |request| request.body = Some(body))
}

#[no_mangle]
pub extern "C" fn mesh_http_body_bytes(handle: u64, body: *const MeshBytes) -> u64 {
    if body.is_null() {
        return 0;
    }
    let body = unsafe { (*body).as_slice().to_vec() };
    update_request(handle, |request| request.body = Some(body))
}

#[no_mangle]
pub extern "C" fn mesh_http_json(handle: u64, body: *const MeshString) -> u64 {
    if body.is_null() {
        return 0;
    }
    let body = unsafe { (*body).as_str().as_bytes().to_vec() };
    update_request(handle, |request| {
        request.body = Some(body);
        request.is_json = true;
    })
}

#[no_mangle]
pub extern "C" fn mesh_http_query(
    handle: u64,
    key: *const MeshString,
    value: *const MeshString,
) -> u64 {
    if key.is_null() || value.is_null() {
        return 0;
    }
    let key = unsafe { (*key).as_str().to_string() };
    let value = unsafe { (*value).as_str().to_string() };
    update_request(handle, |request| request.query_params.push((key, value)))
}

#[no_mangle]
pub extern "C" fn mesh_http_timeout(handle: u64, millis: i64) -> u64 {
    update_request(handle, |request| {
        set_timeout(
            &mut request.timeouts.global,
            &mut request.config_error,
            millis,
            "global",
        )
    })
}

#[no_mangle]
pub extern "C" fn mesh_http_stage_timeout(
    handle: u64,
    stage: *const MeshString,
    millis: i64,
) -> u64 {
    if stage.is_null() {
        return 0;
    }
    let stage = unsafe { (*stage).as_str().to_string() };
    update_request(handle, |request| {
        let timeout = match stage.as_str() {
            "resolve" | "dns" => &mut request.timeouts.resolve,
            "connect" | "tls" => &mut request.timeouts.connect,
            "send" => &mut request.timeouts.send,
            "first_byte" => &mut request.timeouts.first_byte,
            "body" => &mut request.timeouts.body,
            _ => {
                request.config_error = Some(format!(
                    "unknown HTTP timeout stage {stage}; expected resolve, connect, send, first_byte, or body"
                ));
                return;
            }
        };
        set_timeout(timeout, &mut request.config_error, millis, &stage);
    })
}

#[no_mangle]
pub extern "C" fn mesh_http_max_response_bytes(handle: u64, bytes: i64) -> u64 {
    update_request(handle, |request| {
        let Ok(bytes) = usize::try_from(bytes) else {
            request.config_error = Some("maximum response bytes must be positive".to_string());
            return;
        };
        if !(1..=MAX_RESPONSE_BYTES).contains(&bytes) {
            request.config_error = Some(format!(
                "maximum response bytes must be between 1 and {MAX_RESPONSE_BYTES}"
            ));
        } else {
            request.max_response_bytes = bytes;
        }
    })
}

#[no_mangle]
pub extern "C" fn mesh_http_max_redirects(handle: u64, count: i64) -> u64 {
    update_request(handle, |request| {
        let Ok(count) = u32::try_from(count) else {
            request.config_error = Some(format!(
                "maximum redirects must be between 0 and {MAX_REDIRECTS}"
            ));
            return;
        };
        if count > MAX_REDIRECTS {
            request.config_error = Some(format!(
                "maximum redirects must be between 0 and {MAX_REDIRECTS}"
            ));
        } else {
            request.max_redirects = Some(count);
        }
    })
}

fn take_request(handle: u64) -> Result<MeshRequestData, String> {
    if handle == 0 {
        return Err("invalid or already-consumed HTTP request handle".to_string());
    }
    requests()
        .lock()
        .remove(&handle)
        .ok_or_else(|| "invalid or already-consumed HTTP request handle".to_string())
}

fn activate_request(
    handle: u64,
    cancel: Arc<AtomicBool>,
    sender: CooperativeSender<WorkerEvent>,
) -> Result<MeshRequestData, String> {
    if handle == 0 {
        return Err("invalid or already-consumed HTTP request handle".to_string());
    }
    let mut requests = requests().lock();
    let request = requests
        .remove(&handle)
        .ok_or_else(|| "invalid or already-consumed HTTP request handle".to_string())?;
    let mut active = active_requests().lock();
    if active.len() >= MAX_OPEN_HANDLES {
        return Err("too many active HTTP requests".to_string());
    }
    active.insert(handle, ActiveRequest { cancel, sender });
    Ok(request)
}

fn validate_request(request: &MeshRequestData) -> Result<(), String> {
    if let Some(error) = &request.config_error {
        return Err(error.clone());
    }
    if !matches!(
        request.method.as_str(),
        "get" | "head" | "post" | "put" | "patch" | "delete" | "options"
    ) {
        return Err(format!("unsupported HTTP method {}", request.method));
    }
    if request.body.is_some() && !matches!(request.method.as_str(), "post" | "put" | "patch") {
        return Err(format!(
            "HTTP {} requests cannot carry a body",
            request.method
        ));
    }
    Ok(())
}

fn request_url(request: &MeshRequestData) -> Result<String, String> {
    let mut url = Url::parse(&request.url).map_err(|error| format!("INVALID_URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("INVALID_URL: HTTP URL must use http:// or https://".to_string());
    }
    if !request.query_params.is_empty() {
        let mut query = url.query_pairs_mut();
        for (key, value) in &request.query_params {
            query.append_pair(key, value);
        }
    }
    Ok(url.into())
}

fn configure<T>(builder: RequestBuilder<T>, request: &MeshRequestData) -> RequestBuilder<T> {
    let config = builder
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(request.timeouts.global))
        .timeout_resolve(Some(request.timeouts.resolve))
        .timeout_connect(Some(request.timeouts.connect))
        .timeout_send_request(Some(request.timeouts.send))
        .timeout_send_body(Some(request.timeouts.send))
        .timeout_recv_response(Some(request.timeouts.first_byte))
        .timeout_recv_body(Some(request.timeouts.body));
    match request.max_redirects {
        Some(max_redirects) => config.max_redirects(max_redirects).build(),
        None => config.build(),
    }
}

fn apply_headers<T>(
    mut builder: RequestBuilder<T>,
    request: &MeshRequestData,
) -> RequestBuilder<T> {
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    builder
}

fn dispatch(
    agent: &Agent,
    request: &MeshRequestData,
    url: &str,
) -> Result<ureq::http::Response<ureq::Body>, UreqError> {
    match request.method.as_str() {
        "post" => {
            let mut builder = apply_headers(configure(agent.post(url), request), request);
            if request.is_json {
                builder = builder.header("Content-Type", "application/json");
            }
            builder.send(request.body.as_deref().unwrap_or_default())
        }
        "put" => {
            let mut builder = apply_headers(configure(agent.put(url), request), request);
            if request.is_json {
                builder = builder.header("Content-Type", "application/json");
            }
            builder.send(request.body.as_deref().unwrap_or_default())
        }
        "patch" => {
            let mut builder = apply_headers(configure(agent.patch(url), request), request);
            if request.is_json {
                builder = builder.header("Content-Type", "application/json");
            }
            builder.send(request.body.as_deref().unwrap_or_default())
        }
        "get" => apply_headers(configure(agent.get(url), request), request).call(),
        "head" => apply_headers(configure(agent.head(url), request), request).call(),
        "delete" => apply_headers(configure(agent.delete(url), request), request).call(),
        "options" => apply_headers(configure(agent.options(url), request), request).call(),
        _ => unreachable!("request method was validated"),
    }
}

fn duration_micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

struct TotalTimer(Instant);

impl Drop for TotalTimer {
    fn drop(&mut self) {
        metrics()
            .total_micros
            .fetch_add(duration_micros(self.0).max(1), Ordering::Relaxed);
    }
}

fn record_error(error: &UreqError) {
    match error {
        UreqError::HostNotFound => {
            metrics().dns_failures.fetch_add(1, Ordering::Relaxed);
        }
        UreqError::ConnectionFailed | UreqError::ConnectProxyFailed(_) => {
            metrics().connect_failures.fetch_add(1, Ordering::Relaxed);
        }
        UreqError::Tls(_) | UreqError::TlsRequired => {
            metrics().tls_failures.fetch_add(1, Ordering::Relaxed);
        }
        UreqError::Timeout(_) => {
            metrics().timeouts.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
}

fn timeout_prefix(timeout: ureq::Timeout) -> &'static str {
    match timeout {
        ureq::Timeout::Resolve => "TIMEOUT_RESOLVE",
        ureq::Timeout::Connect => "TIMEOUT_CONNECT",
        ureq::Timeout::SendRequest | ureq::Timeout::SendBody => "TIMEOUT_SEND",
        ureq::Timeout::RecvResponse => "TIMEOUT_FIRST_BYTE",
        ureq::Timeout::RecvBody => "TIMEOUT_BODY",
        _ => "TIMEOUT_TOTAL",
    }
}

fn format_error(error: &UreqError) -> String {
    match error {
        UreqError::Timeout(stage) => format!("{}: {error}", timeout_prefix(*stage)),
        UreqError::HostNotFound => format!("DNS_FAILURE: {error}"),
        UreqError::ConnectionFailed | UreqError::ConnectProxyFailed(_) => {
            format!("CONNECT_FAILURE: {error}")
        }
        UreqError::Tls(_) | UreqError::TlsRequired => format!("TLS_ERROR: {error}"),
        UreqError::BodyExceedsLimit(limit) => {
            format!("RESPONSE_TOO_LARGE: limit is {limit} bytes")
        }
        UreqError::BadUri(_) | UreqError::Http(_) => format!("INVALID_REQUEST: {error}"),
        UreqError::Io(reason)
            if matches!(
                reason.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::AddrNotAvailable
            ) =>
        {
            metrics().connect_failures.fetch_add(1, Ordering::Relaxed);
            format!("CONNECT_FAILURE: {error}")
        }
        _ if error.to_string().contains("certificate") || error.to_string().contains("rustls") => {
            metrics().tls_failures.fetch_add(1, Ordering::Relaxed);
            format!("TLS_ERROR: {error}")
        }
        _ => format!("HTTP_ERROR: {error}"),
    }
}

fn execute_request(
    agent: &Agent,
    request: MeshRequestData,
    cancel: Option<&AtomicBool>,
) -> Result<WorkerResponse, String> {
    validate_request(&request)?;
    let url = request_url(&request)?;
    let started = Instant::now();
    let _total = TotalTimer(started);
    let _in_flight = InFlight::begin();
    if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return Err("CANCELLED: HTTP request".to_string());
    }

    let mut response = match dispatch(agent, &request, &url) {
        Ok(response) => response,
        Err(error) => {
            record_error(&error);
            return Err(format_error(&error));
        }
    };
    metrics()
        .first_byte_micros
        .fetch_add(duration_micros(started), Ordering::Relaxed);
    let status = response.status().as_u16() as i64;
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let maximum = request.max_response_bytes;
    let body = response
        .body_mut()
        .with_config()
        .limit(maximum.saturating_add(1) as u64)
        .read_to_vec()
        .map_err(|error| {
            record_error(&error);
            if matches!(error, UreqError::BodyExceedsLimit(_)) {
                format!("RESPONSE_TOO_LARGE: limit is {maximum} bytes")
            } else {
                format_error(&error)
            }
        })?;
    if body.len() > maximum {
        return Err(format!("RESPONSE_TOO_LARGE: limit is {maximum} bytes"));
    }
    if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return Err("CANCELLED: HTTP request".to_string());
    }
    metrics()
        .response_bytes
        .fetch_add(body.len() as u64, Ordering::Relaxed);
    Ok(WorkerResponse {
        status,
        body,
        headers,
    })
}

fn mesh_error(message: impl AsRef<str>) -> *mut u8 {
    let message = message.as_ref();
    alloc_result(
        1,
        mesh_string_new(message.as_ptr(), message.len() as u64).cast(),
    )
    .cast()
}

fn mesh_response(response: WorkerResponse) -> *mut u8 {
    unsafe {
        let body_text = std::str::from_utf8(&response.body).unwrap_or("");
        let body = mesh_string_new(body_text.as_ptr(), body_text.len() as u64);
        let body_bytes = mesh_bytes_new(response.body.as_ptr(), response.body.len() as u64);
        let mut headers = mesh_map_new_typed(1);
        for (name, value) in response.headers {
            let name = mesh_string_new(name.as_ptr(), name.len() as u64);
            let value = mesh_string_new(value.as_ptr(), value.len() as u64);
            headers = mesh_map_put(headers, name as u64, value as u64);
        }
        let output = mesh_gc_alloc_actor(
            std::mem::size_of::<MeshClientResponse>() as u64,
            std::mem::align_of::<MeshClientResponse>() as u64,
        ) as *mut MeshClientResponse;
        (*output).status = response.status;
        (*output).body = body.cast();
        (*output).headers = headers.cast();
        (*output).body_bytes = body_bytes.cast();
        alloc_result(0, output.cast()).cast()
    }
}

fn send_cooperatively(agent: Agent, handle: u64) -> *mut u8 {
    let (sender, receiver) = cooperative_channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let request = match activate_request(handle, Arc::clone(&cancel), sender.clone()) {
        Ok(request) => request,
        Err(error) => return mesh_error(error),
    };
    let wait = request
        .timeouts
        .global
        .saturating_add(Duration::from_secs(1));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::spawn(move || {
        let _ = sender.send(WorkerEvent::Complete(execute_request(
            &agent,
            request,
            Some(&worker_cancel),
        )));
    });
    let result = match cooperative_recv_timeout(&receiver, wait) {
        Ok(WorkerEvent::Complete(Ok(response))) => mesh_response(response),
        Ok(WorkerEvent::Complete(Err(error))) => mesh_error(error),
        Ok(WorkerEvent::Cancelled) => mesh_error("CANCELLED: HTTP request"),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            cancel.store(true, Ordering::Release);
            mesh_error("TIMEOUT_TOTAL: HTTP worker exceeded global timeout")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            cancel.store(true, Ordering::Release);
            mesh_error("HTTP_ERROR: HTTP worker stopped")
        }
    };
    active_requests().lock().remove(&handle);
    result
}

#[no_mangle]
pub extern "C" fn mesh_http_send(handle: u64) -> *mut u8 {
    send_cooperatively(http_agent(), handle)
}

#[no_mangle]
pub extern "C" fn mesh_http_client() -> u64 {
    let mut registry = clients().lock();
    if registry.len() >= MAX_OPEN_HANDLES {
        return 0;
    }
    let handle = next_handle();
    registry.insert(handle, http_agent());
    handle
}

#[no_mangle]
pub extern "C" fn mesh_http_client_close(handle: u64) {
    clients().lock().remove(&handle);
}

#[no_mangle]
pub extern "C" fn mesh_http_send_with(client_handle: u64, request_handle: u64) -> *mut u8 {
    let Some(agent) = clients().lock().get(&client_handle).cloned() else {
        return mesh_error("closed or unknown HTTP client handle");
    };
    send_cooperatively(agent, request_handle)
}

fn is_stop(result: *mut u8) -> bool {
    !result.is_null() && unsafe { (*(result as *const MeshString)).as_str() == "stop" }
}

fn call_stream_callback(callback_fn: usize, callback_env: usize, data: *mut u8) -> bool {
    let result = unsafe {
        if callback_env == 0 {
            let callback: fn(*mut u8) -> *mut u8 = std::mem::transmute(callback_fn);
            callback(data)
        } else {
            let callback: fn(*mut u8, *mut u8) -> *mut u8 = std::mem::transmute(callback_fn);
            callback(callback_env as *mut u8, data)
        }
    };
    is_stop(result)
}

fn emit_text(
    callback_fn: usize,
    callback_env: usize,
    pending: &mut Vec<u8>,
    final_chunk: bool,
) -> Result<bool, String> {
    loop {
        match std::str::from_utf8(pending) {
            Ok(text) => {
                if text.is_empty() {
                    return Ok(false);
                }
                let text = mesh_string_new(text.as_ptr(), text.len() as u64).cast();
                pending.clear();
                return Ok(call_stream_callback(callback_fn, callback_env, text));
            }
            Err(error) if error.valid_up_to() > 0 => {
                let valid = pending.drain(..error.valid_up_to()).collect::<Vec<_>>();
                let text = mesh_string_new(valid.as_ptr(), valid.len() as u64).cast();
                if call_stream_callback(callback_fn, callback_env, text) {
                    return Ok(true);
                }
            }
            Err(error) if error.error_len().is_some() || final_chunk => {
                return Err("INVALID_UTF8: streamed HTTP body".to_string());
            }
            Err(_) => return Ok(false),
        }
    }
}

fn execute_stream(
    agent: Agent,
    request: MeshRequestData,
    cancel: Arc<AtomicBool>,
    callback_fn: usize,
    callback_env: usize,
    binary: bool,
) -> Result<(), String> {
    validate_request(&request)?;
    let url = request_url(&request)?;
    let started = Instant::now();
    let _total = TotalTimer(started);
    let _in_flight = InFlight::begin();
    let mut response = match dispatch(&agent, &request, &url) {
        Ok(response) => response,
        Err(error) => {
            record_error(&error);
            return Err(format_error(&error));
        }
    };
    metrics()
        .first_byte_micros
        .fetch_add(duration_micros(started), Ordering::Relaxed);
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(request.max_response_bytes as u64)
        .reader();
    let mut buffer = vec![0u8; STREAM_CHUNK_BYTES.min(request.max_response_bytes)];
    let mut pending_text = Vec::new();
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("CANCELLED: HTTP stream".to_string());
        }
        let count = reader.read(&mut buffer).map_err(|error| {
            let error = UreqError::from(error);
            record_error(&error);
            format_error(&error)
        })?;
        if count == 0 {
            if !binary {
                emit_text(callback_fn, callback_env, &mut pending_text, true)?;
            }
            break;
        }
        metrics()
            .response_bytes
            .fetch_add(count as u64, Ordering::Relaxed);
        let stopped = if binary {
            let bytes = mesh_bytes_new(buffer.as_ptr(), count as u64).cast();
            call_stream_callback(callback_fn, callback_env, bytes)
        } else {
            pending_text.extend_from_slice(&buffer[..count]);
            emit_text(callback_fn, callback_env, &mut pending_text, false)?
        };
        if stopped {
            break;
        }
    }
    Ok(())
}

fn start_stream(
    request_handle: u64,
    callback_fn: *mut u8,
    callback_env: *mut u8,
    binary: bool,
) -> i64 {
    if callback_fn.is_null() {
        return 0;
    }
    let Ok(request) = take_request(request_handle) else {
        return 0;
    };
    if validate_request(&request).is_err() {
        return 0;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let handle = {
        let mut registry = streams().lock();
        if registry.len() >= MAX_OPEN_HANDLES {
            return 0;
        }
        let handle = next_handle();
        registry.insert(handle, Arc::clone(&cancel));
        handle
    };
    let callback_fn = callback_fn as usize;
    let callback_env = callback_env as usize;
    std::thread::spawn(move || {
        let _ = execute_stream(
            http_agent(),
            request,
            cancel,
            callback_fn,
            callback_env,
            binary,
        );
        streams().lock().remove(&handle);
    });
    i64::try_from(handle).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn mesh_http_stream(
    request_handle: u64,
    callback_fn: *mut u8,
    callback_env: *mut u8,
) -> i64 {
    start_stream(request_handle, callback_fn, callback_env, false)
}

#[no_mangle]
pub extern "C" fn mesh_http_stream_bytes(
    request_handle: u64,
    callback_fn: *mut u8,
    callback_env: *mut u8,
) -> i64 {
    start_stream(request_handle, callback_fn, callback_env, true)
}

#[no_mangle]
pub extern "C" fn mesh_http_cancel(handle: u64) {
    if requests().lock().remove(&handle).is_some() {
        metrics().cancellations.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if let Some(active) = active_requests().lock().remove(&handle) {
        active.cancel.store(true, Ordering::Release);
        let _ = active.sender.send(WorkerEvent::Cancelled);
        metrics().cancellations.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if let Some(cancel) = streams().lock().remove(&handle) {
        cancel.store(true, Ordering::Release);
        metrics().cancellations.fetch_add(1, Ordering::Relaxed);
    }
}

fn retry_class(method: &str, error: &str) -> &'static str {
    let transient = error.starts_with("TIMEOUT_")
        || error.starts_with("DNS_FAILURE:")
        || error.starts_with("CONNECT_FAILURE:");
    if !transient {
        "do_not_retry"
    } else if matches!(
        method.to_ascii_lowercase().as_str(),
        "get" | "head" | "options"
    ) {
        "safe_retry"
    } else {
        "unsafe_retry"
    }
}

#[no_mangle]
pub extern "C" fn mesh_http_retry_class(
    method: *const MeshString,
    error: *const MeshString,
) -> *mut MeshString {
    let class = if method.is_null() || error.is_null() {
        "do_not_retry"
    } else {
        retry_class(unsafe { (*method).as_str() }, unsafe { (*error).as_str() })
    };
    mesh_string_new(class.as_ptr(), class.len() as u64)
}

fn metric_value(value: &AtomicU64) -> i64 {
    i64::try_from(value.load(Ordering::Relaxed)).unwrap_or(i64::MAX)
}

#[no_mangle]
pub extern "C" fn mesh_http_metrics() -> *mut MeshHttpClientMetrics {
    unsafe {
        let output = mesh_gc_alloc_actor(
            std::mem::size_of::<MeshHttpClientMetrics>() as u64,
            std::mem::align_of::<MeshHttpClientMetrics>() as u64,
        ) as *mut MeshHttpClientMetrics;
        (*output).requests = metric_value(&metrics().requests);
        (*output).in_flight = metric_value(&metrics().in_flight);
        (*output).dns_micros = metric_value(&metrics().dns_micros);
        (*output).connect_micros = metric_value(&metrics().connect_micros);
        (*output).tls_micros = metric_value(&metrics().tls_micros);
        (*output).dns_failures = metric_value(&metrics().dns_failures);
        (*output).connect_failures = metric_value(&metrics().connect_failures);
        (*output).tls_failures = metric_value(&metrics().tls_failures);
        (*output).timeouts = metric_value(&metrics().timeouts);
        (*output).first_byte_micros = metric_value(&metrics().first_byte_micros);
        (*output).total_micros = metric_value(&metrics().total_micros);
        (*output).response_bytes = metric_value(&metrics().response_bytes);
        (*output).cancellations = metric_value(&metrics().cancellations);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::MeshBytes;
    use std::io::Write;
    use std::net::TcpListener;

    struct StreamState {
        calls: AtomicU64,
        bytes: AtomicU64,
    }

    fn slow_bytes_callback(environment: *mut u8, chunk: *mut u8) -> *mut u8 {
        let state = unsafe { &*(environment as *const StreamState) };
        let chunk = unsafe { &*(chunk as *const MeshBytes) };
        assert!(chunk.len <= STREAM_CHUNK_BYTES as u64);
        state.calls.fetch_add(1, Ordering::Relaxed);
        state.bytes.fetch_add(chunk.len, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(20));
        std::ptr::null_mut()
    }

    #[test]
    fn response_body_limit_fails_closed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
                .unwrap();
        });
        let mut request = MeshRequestData::new("get", &format!("http://127.0.0.1:{port}/"));
        request.max_response_bytes = 4;

        let error = execute_request(&Agent::new_with_defaults(), request, None).unwrap_err();

        assert!(error.starts_with("RESPONSE_TOO_LARGE:"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn unset_redirect_limit_preserves_agent_default() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_port = target.local_addr().unwrap().port();
        let target_server = std::thread::spawn(move || {
            let (mut stream, _) = target.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nfinal")
                .unwrap();
        });
        let redirect = TcpListener::bind("127.0.0.1:0").unwrap();
        let redirect_port = redirect.local_addr().unwrap().port();
        let redirect_server = std::thread::spawn(move || {
            let (mut stream, _) = redirect.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{target_port}/target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let request =
            MeshRequestData::new("get", &format!("http://127.0.0.1:{redirect_port}/redirect"));

        let response = execute_request(&http_agent(), request, None).unwrap();

        assert_eq!(response.body, b"final");
        redirect_server.join().unwrap();
        target_server.join().unwrap();
    }

    #[test]
    fn redirect_limit_rejects_counts_above_bound() {
        let handle = next_handle();
        requests()
            .lock()
            .insert(handle, MeshRequestData::new("get", "http://example.com"));

        mesh_http_max_redirects(handle, i64::from(MAX_REDIRECTS) + 1);
        let request = take_request(handle).unwrap();

        assert_eq!(
            validate_request(&request).unwrap_err(),
            format!("maximum redirects must be between 0 and {MAX_REDIRECTS}")
        );
    }

    #[test]
    fn first_byte_timeout_is_stage_classified() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        });
        let mut request = MeshRequestData::new("get", &format!("http://127.0.0.1:{port}/"));
        request.timeouts.first_byte = Duration::from_millis(20);

        let started = Instant::now();
        let error = execute_request(&Agent::new_with_defaults(), request, None).unwrap_err();

        assert!(error.starts_with("TIMEOUT_FIRST_BYTE:"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
        server.join().unwrap();
    }

    #[test]
    fn byte_stream_is_bounded_and_callback_backpressured() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let body = vec![7u8; STREAM_CHUNK_BYTES * 2];
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        let request = MeshRequestData::new("get", &format!("http://127.0.0.1:{port}/"));
        let cancel = Arc::new(AtomicBool::new(false));
        let state = Box::new(StreamState {
            calls: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        });
        let started = Instant::now();

        execute_stream(
            http_agent(),
            request,
            cancel,
            slow_bytes_callback as *const () as usize,
            &*state as *const StreamState as usize,
            true,
        )
        .unwrap();

        assert!(state.calls.load(Ordering::Relaxed) >= 2);
        assert_eq!(
            state.bytes.load(Ordering::Relaxed),
            (STREAM_CHUNK_BYTES * 2) as u64
        );
        assert!(started.elapsed() >= Duration::from_millis(40));
        server.join().unwrap();
    }

    #[test]
    fn retry_class_never_automatically_retries_writes() {
        assert_eq!(
            retry_class("get", "TIMEOUT_CONNECT: timed out"),
            "safe_retry"
        );
        assert_eq!(
            retry_class("post", "TIMEOUT_CONNECT: timed out"),
            "unsafe_retry"
        );
        assert_eq!(
            retry_class("get", "TLS_ERROR: invalid certificate"),
            "do_not_retry"
        );
    }

    #[test]
    fn client_and_cancel_handles_are_idempotent() {
        let client = mesh_http_client();
        assert_ne!(client, 0);
        mesh_http_client_close(client);
        mesh_http_client_close(client);
        mesh_http_cancel(0);
    }

    #[test]
    fn in_flight_request_cancellation_returns_without_waiting_for_io() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            accepted_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_secs(1));
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        });
        let method = mesh_string_new(b"get".as_ptr(), 3);
        let url = format!("http://127.0.0.1:{port}/");
        let url = mesh_string_new(url.as_ptr(), url.len() as u64);
        let request = mesh_http_build(method, url);
        let started = Instant::now();
        let send = std::thread::spawn(move || mesh_http_send(request) as usize);

        accepted_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        mesh_http_cancel(request);
        let result = send.join().unwrap() as *mut crate::io::MeshResult;
        let error = unsafe { &*((*result).value as *const MeshString) };

        assert_eq!(unsafe { (*result).tag }, 1);
        assert_eq!(unsafe { error.as_str() }, "CANCELLED: HTTP request");
        assert!(started.elapsed() < Duration::from_millis(500));
        server.join().unwrap();
    }
}
