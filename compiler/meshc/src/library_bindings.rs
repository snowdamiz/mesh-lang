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
            "artifact": artifact.file_name().and_then(|name| name.to_str()),
            "target": target.unwrap_or(std::env::consts::ARCH),
            "ownership": {
                "request": "borrowed-for-call",
                "response": "caller-owned; release with mesh_library_free_returned_bytes"
            },
            "exports": exports.iter().map(|export| serde_json::json!({
                "meshFunction": export.function,
                "symbol": export.symbol,
                "request": "bytes",
                "response": "result<bytes,string>"
            })).collect::<Vec<_>>()
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
    let declarations = exports
        .iter()
        .map(|export| {
            format!(
                "int32_t {}(const uint8_t *request, uint64_t request_len, MeshLibraryBytes *response);",
                export.symbol
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "#ifndef {guard}_H\n#define {guard}_H\n\n#include <stdint.h>\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n#define MESH_LIBRARY_ABI_VERSION {ABI_VERSION}\n#define MESH_LIBRARY_OK 0\n#define MESH_LIBRARY_ERR_INVALID_ARGUMENT 1\n#define MESH_LIBRARY_ERR_NOT_INITIALIZED 2\n#define MESH_LIBRARY_ERR_BUSY 3\n#define MESH_LIBRARY_ERR_PANIC 4\n#define MESH_LIBRARY_ERR_HOST_CALLBACK 5\n#define MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE 6\n#define MESH_LIBRARY_ERR_ABI 7\n#define MESH_LIBRARY_ERR_CALLBACK_MISSING 8\n#define MESH_LIBRARY_ERR_APPLICATION 9\n\ntypedef struct MeshLibraryBytes {{\n  uint8_t *data;\n  uint64_t len;\n}} MeshLibraryBytes;\n\ntypedef int32_t (*MeshLibraryHostCallback)(void *context, const uint8_t *input, uint64_t input_len, uint8_t *output, uint64_t output_capacity, uint64_t *output_len);\n\ntypedef struct MeshLibraryHostCallbacksV1 {{\n  uint32_t abi_version;\n  uint32_t struct_size;\n  void *context;\n  MeshLibraryHostCallback secure_store_put;\n  MeshLibraryHostCallback secure_store_get;\n  MeshLibraryHostCallback secure_store_delete;\n  MeshLibraryHostCallback push_get_token;\n  MeshLibraryHostCallback background_schedule;\n  MeshLibraryHostCallback network_state;\n  MeshLibraryHostCallback monotonic_clock;\n  MeshLibraryHostCallback wall_clock;\n  MeshLibraryHostCallback log_redacted;\n}} MeshLibraryHostCallbacksV1;\n\nint32_t mesh_library_init(void);\nint32_t mesh_library_shutdown(void);\nint32_t mesh_library_register_host_callbacks(const MeshLibraryHostCallbacksV1 *callbacks);\nvoid mesh_library_free_returned_bytes(MeshLibraryBytes *bytes);\n{declarations}\n\n#ifdef __cplusplus\n}}\n#endif\n\n#endif\n"
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
        "import Foundation\n\npublic struct MeshLibraryFailure: Error {{\n  public let status: Int32\n  public let payload: Data\n}}\n\npublic enum MeshLibrary {{\n  public static func initialize() throws {{\n    let status = mesh_library_init()\n    guard status == MESH_LIBRARY_OK else {{ throw MeshLibraryFailure(status: status, payload: Data()) }}\n  }}\n\n  public static func shutdown() {{ _ = mesh_library_shutdown() }}\n\n{functions}\n}}\n"
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
        "package mesh\n\nobject MeshLibrary {{\n    init {{\n        System.loadLibrary(\"{load_name}\")\n        check(initializeNative() == 0)\n    }}\n\n    @JvmStatic private external fun initializeNative(): Int\n    @JvmStatic external fun shutdownNative(): Int\n{declarations}\n}}\n"
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
            format!(
                "JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_{}(JNIEnv *env, jclass cls, jbyteArray request) {{\n  (void)cls;\n  jsize request_len = (*env)->GetArrayLength(env, request);\n  jbyte *request_data = (*env)->GetByteArrayElements(env, request, NULL);\n  MeshLibraryBytes response = {{0}};\n  int32_t status = {}((const uint8_t *)request_data, (uint64_t)request_len, &response);\n  (*env)->ReleaseByteArrayElements(env, request, request_data, JNI_ABORT);\n  if (status != MESH_LIBRARY_OK) {{\n    jclass error = (*env)->FindClass(env, \"java/lang/IllegalStateException\");\n    (*env)->ThrowNew(env, error, \"Mesh library call failed\");\n    mesh_library_free_returned_bytes(&response);\n    return NULL;\n  }}\n  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);\n  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);\n  mesh_library_free_returned_bytes(&response);\n  return result;\n}}",
                export.function, export.symbol
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "#include <jni.h>\n#include \"{header}\"\n\nJNIEXPORT jint JNICALL Java_mesh_MeshLibrary_initializeNative(JNIEnv *env, jclass cls) {{ (void)env; (void)cls; return mesh_library_init(); }}\nJNIEXPORT jint JNICALL Java_mesh_MeshLibrary_shutdownNative(JNIEnv *env, jclass cls) {{ (void)env; (void)cls; return mesh_library_shutdown(); }}\n\n{functions}\n"
    )
}

fn render_typescript(exports: &[LibraryExport]) -> String {
    let functions = exports
        .iter()
        .map(|export| {
            format!(
                "export const {} = (request: Uint8Array): Promise<Uint8Array> => native.invoke('{}', request);",
                export.function, export.symbol
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
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
            &[LibraryExport {
                function: "echo".to_string(),
                symbol: "mesh_mobile_echo".to_string(),
            }],
            Some("aarch64-apple-ios"),
        );
        assert!(bindings.header.contains("mesh_mobile_echo"));
        assert!(bindings
            .swift
            .contains("defer { mesh_library_free_returned_bytes"));
        assert!(bindings.kotlin.contains("external fun echo"));
        assert!(bindings.jni.contains("mesh_library_free_returned_bytes"));
        assert!(bindings.typescript.contains("mesh_mobile_echo"));
        assert!(bindings.manifest.contains("caller-owned"));
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
    }
}
