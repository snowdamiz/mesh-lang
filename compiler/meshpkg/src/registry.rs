use std::io::Read;

use mesh_diagnostics::{Code, Diagnostic};
use ureq::{http::Response, Agent, Body};

pub(crate) fn agent() -> Agent {
    Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

pub(crate) fn response(
    result: Result<Response<Body>, ureq::Error>,
    action: &str,
    publishing: Option<(&str, &str)>,
) -> Result<Response<Body>, Diagnostic> {
    let mut response = result.map_err(|error| {
        Diagnostic::new(Code::RegistryUnreachable, format!("Failed to {action}"))
            .with_note(error.to_string())
    })?;
    let status = response.status().as_u16();
    let success = if publishing.is_some() {
        matches!(status, 200 | 201)
    } else {
        response.status().is_success()
    };
    if success {
        return Ok(response);
    }
    let mut diagnostic = match (status, publishing) {
        (401 | 403, _) => Diagnostic::new(
            Code::RegistryAuthFailed,
            format!("Unauthorized to {action} (HTTP {status})"),
        ),
        (409, Some((name, version))) => Diagnostic::new(
            Code::PublishConflict,
            format!("{name}@{version} already exists in registry. Versions are immutable."),
        ),
        (429 | 500..=599, _) => Diagnostic::new(
            Code::RegistryUnreachable,
            format!("Registry returned HTTP {status} while trying to {action}"),
        ),
        _ => Diagnostic::new(
            Code::RegistryResponseInvalid,
            format!("Registry returned HTTP {status} while trying to {action}"),
        ),
    };
    let mut body = Vec::new();
    if let Err(error) = response
        .body_mut()
        .as_reader()
        .take(16 * 1024)
        .read_to_end(&mut body)
    {
        diagnostic = diagnostic.with_note(format!(
            "Could not read the registry response body: {error}"
        ));
    }
    if !body.is_empty() {
        diagnostic = diagnostic.with_note(format!(
            "Registry response: {}",
            String::from_utf8_lossy(&body)
        ));
    }
    Err(diagnostic)
}

pub(crate) fn json<T: serde::de::DeserializeOwned>(
    mut response: Response<Body>,
    subject: &str,
) -> Result<T, Diagnostic> {
    let body = response.body_mut().read_to_string().map_err(|error| {
        Diagnostic::new(
            Code::RegistryResponseInvalid,
            format!("Failed to read {subject}"),
        )
        .with_note(error.to_string())
    })?;
    serde_json::from_str(&body).map_err(|error| {
        Diagnostic::new(
            Code::RegistryResponseInvalid,
            format!("Failed to parse {subject}"),
        )
        .with_note(error.to_string())
        .with_note(body.chars().take(4096).collect::<String>())
    })
}
