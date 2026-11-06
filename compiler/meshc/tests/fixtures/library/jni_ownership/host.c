#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include "jni.h"
#include "bindings.h"

static const char *mode;
static struct FixtureArray request, result_array;
static const jbyte secret[8] = {1, 2, 3, 4, 5, 6, 7, 8};
static int pending, borrowed, copied, called, released, invalid_set, invalid_throw, owned_input;
static int java_allocations, invalid_new, find_calls, throw_calls, free_calls;
static int allocations, frees, every_free_erased = 1;
struct Allocation { void *pointer; size_t length; int live; };
static struct Allocation owned[8];

static int is_mode(const char *name) { return strcmp(mode, name) == 0; }
static int application_error(void) {
    return is_mode("application-error") || is_mode("error-class-exception") ||
        is_mode("message-allocation-failure") || is_mode("throw-new-exception");
}
static void *fixture_malloc(size_t length) {
    if (is_mode("request-allocation-failure") ||
        (is_mode("message-allocation-failure") && allocations == 1)) return NULL;
    if (allocations == 8) abort();
    void *pointer = malloc(length);
    if (pointer == NULL) abort();
    memset(pointer, 0xa5, length);
    owned[allocations++] = (struct Allocation){pointer, length, 1};
    return pointer;
}
static void fixture_free(void *pointer) {
    if (pointer == NULL) return;
    int found = 0;
    for (int i = 0; i < allocations; ++i) {
        if (owned[i].pointer != pointer || !owned[i].live) continue;
        found = 1;
        const volatile uint8_t *bytes = pointer;
        for (size_t j = 0; j < owned[i].length; ++j) {
            if (bytes[j] != 0) every_free_erased = 0;
        }
        owned[i].live = 0;
        ++frees;
        break;
    }
    if (!found) abort();
    free(pointer);
}

static jsize array_length(JNIEnv *env, jbyteArray value) {
    (void)env;
    if (value == NULL) { pending = 1; return 0; }
    return value->len;
}
static jbyte *borrow_elements(JNIEnv *env, jbyteArray value, jboolean *is_copy) {
    (void)env; (void)is_copy;
    ++borrowed;
    if (value == NULL) { pending = 1; return NULL; }
    if (is_mode("request-copy-exception")) { pending = 1; return NULL; }
    return value->data; /* A permitted direct Java-array alias. */
}
static void release_elements(JNIEnv *env, jbyteArray value, jbyte *data, jint release_mode) {
    (void)env; (void)value; (void)data; (void)release_mode;
}
static void get_region(JNIEnv *env, jbyteArray value, jsize start, jsize length, jbyte *data) {
    (void)env;
    ++copied;
    if (start < 0 || length < 0 || start + length > value->len) abort();
    if (is_mode("request-copy-exception")) {
        if (length > 0) data[0] = value->data[start];
        pending = 1;
        return;
    }
    memcpy(data, value->data + start, (size_t)length);
}
static jbyteArray new_array(JNIEnv *env, jsize length) {
    (void)env;
    ++java_allocations;
    if (is_mode("output-allocation-exception")) { pending = 1; return NULL; }
    if (length < 0 || length > 32) { invalid_new = 1; pending = 1; return NULL; }
    result_array.len = length;
    return &result_array;
}
static void set_region(JNIEnv *env, jbyteArray value, jsize start, jsize length, const jbyte *data) {
    (void)env;
    if (value == NULL || (length != 0 && data == NULL)) { invalid_set = 1; pending = 1; return; }
    if (start < 0 || length < 0 || start + length > value->len) abort();
    if (is_mode("output-copy-exception")) {
        if (length > 0) value->data[start] = data[0];
        pending = 1;
        return;
    }
    memcpy(value->data + start, data, (size_t)length);
}
static jclass find_class(JNIEnv *env, const char *name) {
    (void)env; (void)name;
    ++find_calls;
    if (is_mode("error-class-exception")) { pending = 1; return NULL; }
    return (jclass)(uintptr_t)1;
}
static jint throw_new(JNIEnv *env, jclass cls, const char *message) {
    (void)env;
    ++throw_calls;
    if (cls == NULL) invalid_throw = 1;
    if (message == NULL || strstr(message, "Mesh library call failed") == NULL) abort();
    pending = 1;
    return is_mode("throw-new-exception") ? -1 : 0;
}
static jboolean exception_check(JNIEnv *env) { (void)env; return (jboolean)pending; }
static const struct JNINativeInterface_ functions = {
    array_length, borrow_elements, release_elements, get_region,
    new_array, set_region, find_class, throw_new, exception_check
};

/* These buffers simulate the C producer contract only. The separate real
 * runtime test proves response erasure; this fixture proves every JNI exit
 * invokes that central free. No response-wipe claim is faked here. */
int32_t mesh_test_echo(const uint8_t *bytes, uint64_t length, MeshLibraryBytes *response) {
    ++called;
    owned_input = bytes != (const uint8_t *)request.data;
    if (length > 8 || (length != 0 && (bytes == NULL || memcmp(bytes, secret, (size_t)length) != 0))) {
        return MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE;
    }
    size_t actual_length = is_mode("oversized-response") ? 17 : 3;
    response->len = is_mode("int32-overflow-response") ? (uint64_t)INT32_MAX + 1 : actual_length;
    response->data = is_mode("null-response-data") ? NULL : malloc(actual_length);
    if (response->data == NULL && !is_mode("null-response-data")) abort();
    if (response->data != NULL) {
        memset(response->data, 42, actual_length);
        memcpy(response->data, application_error() ? "bad" : "out", 3);
    }
    return application_error() ? MESH_LIBRARY_ERR_APPLICATION : MESH_LIBRARY_OK;
}
void mesh_library_free_returned_bytes(MeshLibraryBytes *bytes) {
    ++free_calls;
    if (bytes->data != NULL) { ++released; free(bytes->data); }
    *bytes = (MeshLibraryBytes){0};
}
int32_t mesh_library_init(void) { return 0; }
int32_t mesh_library_shutdown(void) { return 0; }

#define malloc fixture_malloc
#define free fixture_free
#include "bindings.jni.c"
#undef malloc
#undef free

int main(int argc, char **argv) {
    if (argc != 2) return 90;
    mode = argv[1];
    request.len = is_mode("empty-request") ? 0 : is_mode("oversized-request") ? 9 : 8;
    memcpy(request.data, secret, sizeof(secret));
    request.data[8] = 42;
    JNIEnv env = &functions;
    pending = is_mode("initial-exception");
    jbyteArray output = Java_mesh_MeshLibrary_echo(&env, NULL, is_mode("null-request") ? NULL : &request);
    int caller_unchanged = memcmp(request.data, secret, sizeof(secret)) == 0 && request.data[8] == 42;
    int no_live_owned = frees == allocations;
    int valid;
    if (is_mode("ordinary-protocol")) {
        valid = called == 1 && released == 1 && output != NULL && !pending &&
            output->len == 3 && memcmp(output->data, "out", 3) == 0 && caller_unchanged;
    } else if (is_mode("owned-success")) {
        valid = called == 1 && released == 1 && output != NULL && !pending && owned_input &&
            borrowed == 0 && copied == 1 && allocations == 1 && no_live_owned && every_free_erased && caller_unchanged;
    } else if (is_mode("empty-request")) {
        valid = called == 1 && released == 1 && output != NULL && !pending &&
            borrowed == 0 && copied == 0 && allocations == 0 && owned_input && caller_unchanged;
    } else if (is_mode("oversized-request")) {
        valid = called == 0 && borrowed == 0 && copied == 0 && output == NULL && pending &&
            no_live_owned && every_free_erased && caller_unchanged;
    } else if (is_mode("request-allocation-failure")) {
        valid = called == 0 && borrowed == 0 && output == NULL && pending &&
            allocations == 0 && caller_unchanged;
    } else if (is_mode("request-copy-exception")) {
        valid = called == 0 && borrowed == 0 && copied == 1 && output == NULL && pending &&
            allocations == 1 && no_live_owned && every_free_erased && caller_unchanged && throw_calls == 0;
    } else if (is_mode("output-allocation-exception")) {
        valid = called == 1 && released == 1 && output == NULL && pending && !invalid_set &&
            no_live_owned && every_free_erased && caller_unchanged && throw_calls == 0;
    } else if (is_mode("output-copy-exception")) {
        valid = called == 1 && released == 1 && output == NULL && pending &&
            no_live_owned && every_free_erased && caller_unchanged && throw_calls == 0;
    } else if (is_mode("initial-exception")) {
        valid = called == 0 && borrowed == 0 && copied == 0 && output == NULL && pending &&
            allocations == 0 && caller_unchanged && find_calls == 0 && throw_calls == 0;
    } else if (is_mode("null-request")) {
        valid = called == 0 && borrowed == 0 && copied == 0 && output == NULL && pending &&
            no_live_owned && every_free_erased && caller_unchanged;
    } else if (is_mode("error-class-exception")) {
        valid = called == 1 && released == 1 && output == NULL && pending && !invalid_throw &&
            allocations == 1 && no_live_owned && every_free_erased && caller_unchanged;
    } else if (is_mode("message-allocation-failure")) {
        valid = called == 1 && released == 1 && output == NULL && pending &&
            allocations == 1 && no_live_owned && every_free_erased && caller_unchanged;
    } else if (is_mode("oversized-response") || is_mode("int32-overflow-response") || is_mode("null-response-data")) {
        valid = called == 1 && free_calls == 1 && output == NULL && pending && java_allocations == 0 && !invalid_new && !invalid_set &&
            no_live_owned && every_free_erased && caller_unchanged;
    } else if (is_mode("throw-new-exception")) {
        valid = called == 1 && released == 1 && output == NULL && pending && throw_calls == 1 &&
            allocations == 2 && no_live_owned && every_free_erased && caller_unchanged;
    } else if (is_mode("application-error")) {
        valid = called == 1 && released == 1 && output == NULL && pending &&
            allocations == 2 && no_live_owned && every_free_erased && caller_unchanged;
    } else {
        return 91;
    }
    printf("JNI_OWNERSHIP_CASE %s valid=%d calls=%d borrowed=%d copied=%d allocations=%d frees=%d erased=%d response_frees=%d caller_preserved=%d java_allocations=%d free_calls=%d throws=%d\n",
        mode, valid, called, borrowed, copied, allocations, frees, every_free_erased, released, caller_unchanged, java_allocations, free_calls, throw_calls);
    return valid ? 0 : 1;
}
