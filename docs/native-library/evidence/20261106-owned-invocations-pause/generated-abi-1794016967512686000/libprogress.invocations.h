#ifndef LIBPROGRESS_H_INVOCATIONS_H
#define LIBPROGRESS_H_INVOCATIONS_H

#include "libprogress.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Native-only opt-in. Require scheduler capability before qualified use.
 * Pin module/runtime/callback code until process exit. Shutdown joins the
 * scheduler, not detached HTTP; it is not an unload or publication fence.
 * Submit copies bounded input and returns one owned handle on status 0.
 * Every failure clears valid output. A handle is lifetime, not permission.
 * Take requires an empty destination, consumes terminal status once, and
 * returns private bytes only on success. Free bytes with the legacy free API.
 * Keep the caller reference alive during concurrent take/cancel; exclude
 * release from those uses. Release does not cancel or wait for execution.
 * Callback/nested Mesh take/cancel returns BUSY without mutating the handle.
 * See docs/native-library/owned-invocations.md for the full status contract. */
#define MESH_LIBRARY_ERR_PENDING 12
#define MESH_LIBRARY_ERR_TAKEN 13
typedef struct MeshOwnedInvocationV2 MeshOwnedInvocationV2;
int32_t mesh_library_invocation_take_v2(MeshOwnedInvocationV2 *, MeshLibraryBytes *);
int32_t mesh_library_invocation_cancel_v2(MeshOwnedInvocationV2 *);
void mesh_library_invocation_release_v2(MeshOwnedInvocationV2 *);
int32_t progress_old_submit_v2(const MeshNativeAuthority *, const uint8_t *, uint64_t, MeshOwnedInvocationV2 **);
int32_t progress_error_submit_v2(const MeshNativeAuthority *, const uint8_t *, uint64_t, MeshOwnedInvocationV2 **);
int32_t progress_load_submit_v2(const MeshNativeAuthority *, const uint8_t *, uint64_t, MeshOwnedInvocationV2 **);
int32_t progress_status_submit_v2(const MeshNativeAuthority *, const uint8_t *, uint64_t, MeshOwnedInvocationV2 **);

#ifdef __cplusplus
}
#endif
#endif
