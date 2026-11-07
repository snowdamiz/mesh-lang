#ifndef LIBPROGRESS_H
#define LIBPROGRESS_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define MESH_LIBRARY_ABI_VERSION 1
#define MESH_LIBRARY_OK 0
#define MESH_LIBRARY_ERR_INVALID_ARGUMENT 1
#define MESH_LIBRARY_ERR_NOT_INITIALIZED 2
#define MESH_LIBRARY_ERR_BUSY 3
#define MESH_LIBRARY_ERR_PANIC 4
#define MESH_LIBRARY_ERR_HOST_CALLBACK 5
#define MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE 6
#define MESH_LIBRARY_ERR_ABI 7
#define MESH_LIBRARY_ERR_CALLBACK_MISSING 8
#define MESH_LIBRARY_ERR_APPLICATION 9

typedef struct MeshLibraryBytes {
  uint8_t *data;
  uint64_t len;
} MeshLibraryBytes;

typedef int32_t (*MeshLibraryHostCallback)(void *context, const uint8_t *input, uint64_t input_len, uint8_t *output, uint64_t output_capacity, uint64_t *output_len);

typedef struct MeshLibraryHostCallbacksV1 {
  uint32_t abi_version;
  uint32_t struct_size;
  void *context;
  MeshLibraryHostCallback secure_store_put;
  MeshLibraryHostCallback secure_store_get;
  MeshLibraryHostCallback secure_store_delete;
  MeshLibraryHostCallback push_get_token;
  MeshLibraryHostCallback background_schedule;
  MeshLibraryHostCallback network_state;
  MeshLibraryHostCallback monotonic_clock;
  MeshLibraryHostCallback wall_clock;
  MeshLibraryHostCallback log_redacted;
} MeshLibraryHostCallbacksV1;

int32_t mesh_library_init(void);
int32_t mesh_library_shutdown(void);
int32_t mesh_library_register_host_callbacks(const MeshLibraryHostCallbacksV1 *callbacks);
void mesh_library_free_returned_bytes(MeshLibraryBytes *bytes);
/* Synchronous native-only extension. V1's exact nine-callback table is unchanged.
 * UNIT1 is a bounded embedding capability, not complete effect/native/publication
 * coverage. An embedding must require every feature its generated inventory uses.
 * Unsupported bits fail before activation; enabling requires pre-init startup.
 */
#define MESH_LIBRARY_AUTHORITY_ABI_VERSION 2
#define MESH_LIBRARY_ERR_AUTHORITY_REFUSED 10
#define MESH_LIBRARY_ERR_UNSUPPORTED_CAPABILITY 11
#define MESH_LIBRARY_CAP_SYNCHRONOUS_SINKS UINT64_C(1)
#define MESH_LIBRARY_CAP_COMPLETE_EFFECTS (UINT64_C(1) << 1)
#define MESH_LIBRARY_CAP_SCHEDULER_INVOCATIONS (UINT64_C(1) << 2)
#define MESH_LIBRARY_CAP_PRIVATE_PUBLICATION (UINT64_C(1) << 3)
#define MESH_LIBRARY_CAP_SERVICE_CONTEXT (UINT64_C(1) << 4)
#define MESH_LIBRARY_CAP_NATIVE_POLICY (UINT64_C(1) << 5)
#define MESH_LIBRARY_MAX_AUTHORITY_ANCESTRY 32

typedef struct MeshNativeDomain MeshNativeDomain;
typedef struct MeshNativeGeneration MeshNativeGeneration;
typedef struct MeshNativeAuthority MeshNativeAuthority;
typedef void (*MeshNativeLifetime)(void *context);
typedef struct MeshLibraryHostCallbacksV2 {
  uint32_t abi_version;
  uint32_t struct_size;
  MeshLibraryHostCallbacksV1 callbacks;
  MeshNativeLifetime retain;
  MeshNativeLifetime release;
} MeshLibraryHostCallbacksV2;
typedef int32_t (*MeshGeneratedExportV2)(const uint8_t *, uint64_t, MeshLibraryBytes *);

uint64_t mesh_library_capabilities_v2(void);
int32_t mesh_library_enable_protected_v2(uint64_t required);
int32_t mesh_authority_domain_create_v2(const MeshNativeDomain **output);
int32_t mesh_authority_generation_v2(const MeshNativeDomain *, const MeshNativeGeneration **output);
int32_t mesh_authority_bind_v2(const MeshNativeDomain *, const MeshNativeGeneration *const *nodes,
  uint64_t node_count, const MeshLibraryHostCallbacksV2 *, const MeshNativeAuthority **output);
/* Retirement is one atomic tombstone store; it neither drains nor joins work. */
int32_t mesh_authority_retire_v2(const MeshNativeGeneration *);
/* After retirement, zero means all admitted local writes have settled. This
 * observation grants no authority and excludes HTTP. Never use a timeout as zero. */
uint64_t mesh_authority_local_inflight_v2(const MeshNativeGeneration *);
void mesh_authority_domain_release_v2(const MeshNativeDomain *);
void mesh_authority_generation_release_v2(const MeshNativeGeneration *);
void mesh_authority_release_v2(const MeshNativeAuthority *);
/* Trusted native adapter validates/narrows the generated descriptor BEFORE
 * passing its function pointer. Request bytes cannot select this pointer.
 * Existing synchronous CALL_LOCK behavior remains. Response is owned runtime
 * output, not permission to publish it to an IPC/UI recipient. */
int32_t mesh_library_call_scoped_v2(const MeshNativeAuthority *, MeshGeneratedExportV2,
  const uint8_t *input, uint64_t input_len, MeshLibraryBytes *output);
/* Diagnostic identity only. Zero outside a scope; never an admission ticket. */
uint64_t mesh_library_current_invocation_v2(void);

/* Request limit: 1048576 bytes. Response/error limit: 1048576 bytes. */
int32_t progress_old(const uint8_t *request, uint64_t request_len, MeshLibraryBytes *response);
/* Request limit: 1048576 bytes. Response/error limit: 1048576 bytes. */
int32_t progress_error(const uint8_t *request, uint64_t request_len, MeshLibraryBytes *response);
/* Request limit: 1048576 bytes. Response/error limit: 1048576 bytes. */
int32_t progress_load(const uint8_t *request, uint64_t request_len, MeshLibraryBytes *response);
/* Request limit: 1048576 bytes. Response/error limit: 1048576 bytes. */
int32_t progress_status(const uint8_t *request, uint64_t request_len, MeshLibraryBytes *response);

#ifdef __cplusplus
}
#endif

#endif
