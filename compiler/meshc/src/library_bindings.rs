use std::path::Path;

use mesh_codegen::LibraryExport;

const ABI_VERSION: u32 = 1;

pub(crate) fn write(
    artifact: &Path,
    exports: &[LibraryExport],
    target: Option<&str>,
) -> Result<(), String> {
    let bindings = render(artifact, exports, target);
    for (extension, contents) in [
        ("h", bindings.header),
        ("swift", bindings.swift),
        ("kt", bindings.kotlin),
        ("jni.c", bindings.jni),
        ("ts", bindings.typescript),
        ("abi.json", bindings.manifest),
    ] {
        let path = artifact.with_extension(extension);
        std::fs::write(&path, contents)
            .map_err(|error| format!("Failed to write '{}': {error}", path.display()))?;
    }
    Ok(())
}

struct Bindings {
    header: String,
    swift: String,
    kotlin: String,
    jni: String,
    typescript: String,
    manifest: String,
}

fn render(artifact: &Path, exports: &[LibraryExport], target: Option<&str>) -> Bindings {
    let target = mesh_codegen::link::effective_target_triple(target)
        .unwrap_or_else(|_| target.unwrap_or(std::env::consts::ARCH).to_string());
    let stem = artifact
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("mesh_library");
    let load_name = stem.strip_prefix("lib").unwrap_or(stem);
    Bindings {
        header: render_header(stem, exports),
        swift: render_swift(exports),
        kotlin: render_kotlin(load_name, exports),
        jni: render_jni(artifact, exports),
        typescript: render_typescript(exports),
        manifest: serde_json::to_string_pretty(&serde_json::json!({
            "abiVersion": ABI_VERSION,
            "nativeAuthorityExtension": {
                "abiVersion": 2,
                "capabilitiesSymbol": "mesh_library_capabilities_v2",
                "activationSymbol": "mesh_library_enable_protected_v2",
                "invocationSymbol": "mesh_library_call_scoped_v2",
                "refusalStatus": 10,
                "unsupportedStatus": 11,
                "completeEffectInventory": false,
                "dispatchTrust": "native descriptor validation required",
                "scheduling": "synchronous; existing call serialization",
                "publication": "not qualified"
            },
            "artifact": artifact.file_name().and_then(|name| name.to_str()),
            "target": target,
            "ownership": {
                "request": "borrowed-for-call",
                "response": "caller-owned; release with mesh_library_free_returned_bytes"
            },
            "exports": exports.iter().map(|export| {
                let mut row = serde_json::json!({
                    "meshFunction": export.function,
                    "symbol": export.symbol,
                    "request": "bytes",
                    "response": "result<bytes,string>",
                    "maxInputBytes": export.max_input,
                    "maxOutputBytes": export.max_output
                });
                if export.binding_audience == mesh_codegen::ExportBindingAudience::Native {
                    row["bindingAudience"] = serde_json::json!("native");
                }
                row
            }).collect::<Vec<_>>()
        }))
        .expect("ABI manifest is serializable")
            + "\n",
    }
}

fn render_header(stem: &str, exports: &[LibraryExport]) -> String {
    let guard = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let authority = include_str!("library_authority.h");
    let declarations = exports
        .iter()
        .map(|export| {
            format!(
                "/* Request limit: {} bytes. Response/error limit: {} bytes. */\nint32_t {}(const uint8_t *request, uint64_t request_len, MeshLibraryBytes *response);",
                export.max_input, export.max_output, export.symbol
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "#ifndef {guard}_H\n#define {guard}_H\n\n#include <stdint.h>\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n#define MESH_LIBRARY_ABI_VERSION {ABI_VERSION}\n#define MESH_LIBRARY_OK 0\n#define MESH_LIBRARY_ERR_INVALID_ARGUMENT 1\n#define MESH_LIBRARY_ERR_NOT_INITIALIZED 2\n#define MESH_LIBRARY_ERR_BUSY 3\n#define MESH_LIBRARY_ERR_PANIC 4\n#define MESH_LIBRARY_ERR_HOST_CALLBACK 5\n#define MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE 6\n#define MESH_LIBRARY_ERR_ABI 7\n#define MESH_LIBRARY_ERR_CALLBACK_MISSING 8\n#define MESH_LIBRARY_ERR_APPLICATION 9\n\ntypedef struct MeshLibraryBytes {{\n  uint8_t *data;\n  uint64_t len;\n}} MeshLibraryBytes;\n\ntypedef int32_t (*MeshLibraryHostCallback)(void *context, const uint8_t *input, uint64_t input_len, uint8_t *output, uint64_t output_capacity, uint64_t *output_len);\n\ntypedef struct MeshLibraryHostCallbacksV1 {{\n  uint32_t abi_version;\n  uint32_t struct_size;\n  void *context;\n  MeshLibraryHostCallback secure_store_put;\n  MeshLibraryHostCallback secure_store_get;\n  MeshLibraryHostCallback secure_store_delete;\n  MeshLibraryHostCallback push_get_token;\n  MeshLibraryHostCallback background_schedule;\n  MeshLibraryHostCallback network_state;\n  MeshLibraryHostCallback monotonic_clock;\n  MeshLibraryHostCallback wall_clock;\n  MeshLibraryHostCallback log_redacted;\n}} MeshLibraryHostCallbacksV1;\n\nint32_t mesh_library_init(void);\nint32_t mesh_library_shutdown(void);\nint32_t mesh_library_register_host_callbacks(const MeshLibraryHostCallbacksV1 *callbacks);\nvoid mesh_library_free_returned_bytes(MeshLibraryBytes *bytes);\n{authority}\n{declarations}\n\n#ifdef __cplusplus\n}}\n#endif\n\n#endif\n"
    )
}

fn render_swift(exports: &[LibraryExport]) -> String {
    let functions = exports
        .iter()
        .map(|export| {
            format!(
                "  public static func {}(_ request: Data) throws -> Data {{\n    var response = MeshLibraryBytes(data: nil, len: 0)\n    let status = request.withUnsafeBytes {{ bytes in\n      {}(bytes.bindMemory(to: UInt8.self).baseAddress, UInt64(bytes.count), &response)\n    }}\n    defer {{ mesh_library_free_returned_bytes(&response) }}\n    let payload = response.len == 0 ? Data() : Data(bytes: response.data!, count: Int(response.len))\n    guard status == MESH_LIBRARY_OK else {{ throw MeshLibraryFailure(status: status, payload: payload) }}\n    return payload\n  }}",
                export.function, export.symbol
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "import Foundation\n\npublic struct MeshLibraryFailure: LocalizedError {{\n  public let status: Int32\n  public let payload: Data\n\n  public var errorDescription: String? {{\n    let summary = \"Mesh library call failed (status=\\(status))\"\n    guard let message = String(data: payload, encoding: .utf8), !message.isEmpty else {{ return summary }}\n    return \"\\(summary): \\(message)\"\n  }}\n}}\n\npublic enum MeshLibrary {{\n  public static func initialize() throws {{\n    let status = mesh_library_init()\n    guard status == MESH_LIBRARY_OK else {{ throw MeshLibraryFailure(status: status, payload: Data()) }}\n  }}\n\n  public static func shutdown() {{ _ = mesh_library_shutdown() }}\n\n{functions}\n}}\n"
    )
}

fn render_kotlin(load_name: &str, exports: &[LibraryExport]) -> String {
    let declarations = exports
        .iter()
        .map(|export| {
            format!(
                "    @JvmStatic external fun {}(request: ByteArray): ByteArray",
                export.function
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "package mesh\n\nobject MeshLibrary {{\n    init {{\n        System.loadLibrary(\"{load_name}\")\n        val status = initializeNative()\n        check(status == 0) {{ \"Mesh library initialization failed (status=$status)\" }}\n    }}\n\n    @JvmStatic fun ensureInitialized() = Unit\n    @JvmStatic private external fun initializeNative(): Int\n    @JvmStatic external fun shutdownNative(): Int\n{declarations}\n}}\n"
    )
}

fn render_jni(artifact: &Path, exports: &[LibraryExport]) -> String {
    let header = artifact
        .with_extension("h")
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mesh_library.h")
        .to_string();
    let functions = exports
        .iter()
        .map(|export| {
            let jni_function = export.function.replace('_', "_1");
            format!(
                r#"JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_{}(JNIEnv *env, jclass cls, jbyteArray request) {{
  (void)cls;
  MeshLibraryBytes response = {{0}};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {{
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }}
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > {}ULL) {{
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }}
  uint8_t *request_data = NULL;
  if (request_len != 0) {{
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {{
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }}
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {{
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }}
  }}
  int32_t status = {}((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {{
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }}
  if (response.len > {}ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {{
    MeshLibraryBytes invalid_response = {{0}};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }}
  if (status != MESH_LIBRARY_OK) {{
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }}
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {{
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }}
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}}"#,
                jni_function, export.max_input, export.symbol, export.max_output
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        r#"#include <jni.h>
#include <stdio.h>
#include <stdlib.h>
#include "{header}"

static void mesh_erase_and_free(void *allocation, size_t length) {{
  if (allocation == NULL) return;
  volatile uint8_t *bytes = (volatile uint8_t *)allocation;
  while (length != 0) {{ *bytes++ = 0; --length; }}
  free(allocation);
}}

static void mesh_throw_library_failure(JNIEnv *env, int32_t status, const MeshLibraryBytes *response) {{
  if ((*env)->ExceptionCheck(env)) return;
  jclass error = (*env)->FindClass(env, "java/lang/IllegalStateException");
  if (error == NULL || (*env)->ExceptionCheck(env)) return;
  size_t payload_len = response->data == NULL ? 0 : response->len > 1048576 ? 1048576 : (size_t)response->len;
  size_t message_len = payload_len + 64;
  char *message = (char *)malloc(message_len);
  if (message == NULL) {{
    (*env)->ThrowNew(env, error, "Mesh library call failed");
    return;
  }}
  snprintf(message, message_len, "Mesh library call failed (status=%d): %.*s", (int)status, (int)payload_len, response->data == NULL ? "" : (const char *)response->data);
  (*env)->ThrowNew(env, error, message);
  mesh_erase_and_free(message, message_len);
}}

JNIEXPORT jint JNICALL Java_mesh_MeshLibrary_initializeNative(JNIEnv *env, jclass cls) {{ (void)env; (void)cls; return mesh_library_init(); }}
JNIEXPORT jint JNICALL Java_mesh_MeshLibrary_shutdownNative(JNIEnv *env, jclass cls) {{ (void)env; (void)cls; return mesh_library_shutdown(); }}

{functions}
"#
    )
}

fn render_typescript(exports: &[LibraryExport]) -> String {
    let functions = exports
        .iter()
        .filter(|export| export.binding_audience == mesh_codegen::ExportBindingAudience::All)
        .map(|export| {
            format!(
                "export const {} = (request: Uint8Array): Promise<Uint8Array> => native.invoke('{}', request);",
                export.function, export.symbol
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if functions.is_empty() {
        return "// No JavaScript bindings were requested.\nexport {};\n".to_string();
    }
    format!(
        "import {{ requireNativeModule }} from 'expo-modules-core';\n\ntype MeshNativeModule = {{\n  invoke(symbol: string, request: Uint8Array): Promise<Uint8Array>;\n}};\n\nconst native = requireNativeModule<MeshNativeModule>('MeshMessenger');\n\n{functions}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_bindings_share_one_ownership_contract() {
        let bindings = render(
            Path::new("/tmp/libmessenger.a"),
            &[
                LibraryExport {
                    function: "echo".to_string(),
                    symbol: "mesh_mobile_echo".to_string(),
                    max_input: 16_782_000,
                    max_output: 1_048_576,
                    binding_audience: mesh_codegen::ExportBindingAudience::All,
                },
                LibraryExport {
                    function: "validate_outer".to_string(),
                    symbol: "mesh_mobile_validate_outer".to_string(),
                    max_input: 1_048_576,
                    max_output: 1_048_576,
                    binding_audience: mesh_codegen::ExportBindingAudience::All,
                },
            ],
            Some("aarch64-apple-ios"),
        );
        assert!(bindings.header.contains("mesh_mobile_echo"));
        assert!(bindings
            .swift
            .contains("defer { mesh_library_free_returned_bytes"));
        assert!(bindings.kotlin.contains("external fun echo"));
        assert!(bindings.kotlin.contains("fun ensureInitialized()"));
        assert!(bindings.jni.contains("mesh_library_free_returned_bytes"));
        assert!(bindings
            .jni
            .contains("Java_mesh_MeshLibrary_validate_1outer"));
        assert!(bindings.jni.contains("status=%d"));
        assert!(bindings.jni.contains("response->data"));
        assert!(bindings.typescript.contains("mesh_mobile_echo"));
        assert!(bindings.manifest.contains("caller-owned"));
        assert!(bindings
            .header
            .contains("Request limit: 16782000 bytes. Response/error limit: 1048576 bytes."));
        let manifest: serde_json::Value = serde_json::from_str(&bindings.manifest).unwrap();
        assert_eq!(manifest["exports"][0]["maxInputBytes"], 16_782_000);
        assert_eq!(manifest["exports"][0]["maxOutputBytes"], 1_048_576);
        for status in [
            "MESH_LIBRARY_ERR_INVALID_ARGUMENT",
            "MESH_LIBRARY_ERR_NOT_INITIALIZED",
            "MESH_LIBRARY_ERR_BUSY",
            "MESH_LIBRARY_ERR_PANIC",
            "MESH_LIBRARY_ERR_HOST_CALLBACK",
            "MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE",
            "MESH_LIBRARY_ERR_ABI",
            "MESH_LIBRARY_ERR_CALLBACK_MISSING",
            "MESH_LIBRARY_ERR_APPLICATION",
        ] {
            assert!(bindings.header.contains(status), "missing {status}");
        }

        let host = mesh_codegen::link::effective_target_triple(None).unwrap();
        let host_bindings = render(Path::new("/tmp/libmessenger.a"), &[], None);
        assert!(host_bindings.manifest.contains(&host));
    }
    #[cfg(unix)]
    fn run_jni_ownership_cases(cases: &[&str]) {
        let fixture = tempfile::tempdir().expect("owned JNI fixture directory");
        let root = fixture.path();
        let bindings = render(
            &root.join("bindings.a"),
            &[LibraryExport {
                function: "echo".to_string(),
                symbol: "mesh_test_echo".to_string(),
                max_input: 8,
                max_output: 16,
                binding_audience: mesh_codegen::ExportBindingAudience::All,
            }],
            None,
        );
        for (name, contents) in [
            ("bindings.h", bindings.header.as_str()),
            ("bindings.jni.c", bindings.jni.as_str()),
            (
                "jni.h",
                include_str!("../tests/fixtures/library/jni_ownership/jni.h"),
            ),
            (
                "host.c",
                include_str!("../tests/fixtures/library/jni_ownership/host.c"),
            ),
        ] {
            std::fs::write(root.join(name), contents).expect("write owned JNI fixture");
        }
        let executable = root.join("jni-ownership");
        let compile =
            std::process::Command::new(std::env::var_os("CC").unwrap_or_else(|| "cc".into()))
                .args(["-std=c11", "-O2", "-Wall", "-Wextra", "-Werror", "-I"])
                .arg(root)
                .arg(root.join("host.c"))
                .arg("-o")
                .arg(&executable)
                .output()
                .expect("run real C compiler for generated JNI fixture");
        assert!(
            compile.status.success(),
            "generated JNI C fixture did not compile: {}{}",
            String::from_utf8_lossy(&compile.stdout),
            String::from_utf8_lossy(&compile.stderr)
        );
        eprintln!("JNI_OWNERSHIP_COMPILED");
        let outcomes: Vec<_> = cases
            .iter()
            .map(|case| {
                let result = std::process::Command::new(&executable)
                    .arg(case)
                    .output()
                    .expect("execute compiled generated JNI ownership fixture");
                eprintln!(
                    "JNI_OWNERSHIP_RESULT {}",
                    serde_json::json!({
                        "case": case,
                        "exit": result.status.code(),
                        "stdout": String::from_utf8_lossy(&result.stdout),
                        "stderr": String::from_utf8_lossy(&result.stderr),
                    })
                );
                (
                    *case,
                    result.status.code(),
                    String::from_utf8_lossy(&result.stdout).into_owned(),
                    String::from_utf8_lossy(&result.stderr).into_owned(),
                )
            })
            .collect();
        assert!(
            outcomes.iter().all(|(case, status, stdout, stderr)| {
                *status == Some(0)
                    && stdout.starts_with(&format!("JNI_OWNERSHIP_CASE {case} valid=1 "))
                    && stdout.lines().count() == 1
                    && stderr.is_empty()
            }),
            "actual generated JNI ownership outcomes: {outcomes:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn generated_jni_request_copy_is_owned_bounded_and_erased_on_every_exit() {
        run_jni_ownership_cases(&[
            "owned-success",
            "empty-request",
            "oversized-request",
            "request-allocation-failure",
            "request-copy-exception",
            "initial-exception",
            "null-request",
        ]);
    }

    #[test]
    #[cfg(unix)]
    fn generated_jni_responses_are_released_on_output_jni_exceptions() {
        run_jni_ownership_cases(&[
            "output-allocation-exception",
            "output-copy-exception",
            "oversized-response",
            "int32-overflow-response",
            "null-response-data",
        ]);
    }

    #[test]
    #[cfg(unix)]
    fn generated_jni_formatted_application_error_is_erased_before_free() {
        run_jni_ownership_cases(&[
            "application-error",
            "error-class-exception",
            "message-allocation-failure",
            "throw-new-exception",
        ]);
    }

    #[test]
    #[cfg(unix)]
    fn generated_jni_ordinary_response_protocol_is_preserved() {
        run_jni_ownership_cases(&["ordinary-protocol"]);
    }
}
