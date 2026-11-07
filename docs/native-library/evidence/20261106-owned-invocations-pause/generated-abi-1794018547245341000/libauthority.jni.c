#include <jni.h>
#include <stdio.h>
#include <stdlib.h>
#include "libauthority.h"

static void mesh_erase_and_free(void *allocation, size_t length) {
  if (allocation == NULL) return;
  volatile uint8_t *bytes = (volatile uint8_t *)allocation;
  while (length != 0) { *bytes++ = 0; --length; }
  free(allocation);
}

static void mesh_throw_library_failure(JNIEnv *env, int32_t status, const MeshLibraryBytes *response) {
  if ((*env)->ExceptionCheck(env)) return;
  jclass error = (*env)->FindClass(env, "java/lang/IllegalStateException");
  if (error == NULL || (*env)->ExceptionCheck(env)) return;
  size_t payload_len = response->data == NULL ? 0 : response->len > 1048576 ? 1048576 : (size_t)response->len;
  size_t message_len = payload_len + 64;
  char *message = (char *)malloc(message_len);
  if (message == NULL) {
    (*env)->ThrowNew(env, error, "Mesh library call failed");
    return;
  }
  snprintf(message, message_len, "Mesh library call failed (status=%d): %.*s", (int)status, (int)payload_len, response->data == NULL ? "" : (const char *)response->data);
  (*env)->ThrowNew(env, error, message);
  mesh_erase_and_free(message, message_len);
}

JNIEXPORT jint JNICALL Java_mesh_MeshLibrary_initializeNative(JNIEnv *env, jclass cls) { (void)env; (void)cls; return mesh_library_init(); }
JNIEXPORT jint JNICALL Java_mesh_MeshLibrary_shutdownNative(JNIEnv *env, jclass cls) { (void)env; (void)cls; return mesh_library_shutdown(); }

JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_sql(JNIEnv *env, jclass cls, jbyteArray request) {
  (void)cls;
  MeshLibraryBytes response = {0};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > 1048576ULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }
  uint8_t *request_data = NULL;
  if (request_len != 0) {
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }
  }
  int32_t status = authority_sql((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len > 1048576ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {
    MeshLibraryBytes invalid_response = {0};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (status != MESH_LIBRARY_OK) {
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}

JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_http(JNIEnv *env, jclass cls, jbyteArray request) {
  (void)cls;
  MeshLibraryBytes response = {0};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > 1048576ULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }
  uint8_t *request_data = NULL;
  if (request_len != 0) {
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }
  }
  int32_t status = authority_http((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len > 1048576ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {
    MeshLibraryBytes invalid_response = {0};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (status != MESH_LIBRARY_OK) {
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}

JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_store(JNIEnv *env, jclass cls, jbyteArray request) {
  (void)cls;
  MeshLibraryBytes response = {0};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > 1048576ULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }
  uint8_t *request_data = NULL;
  if (request_len != 0) {
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }
  }
  int32_t status = authority_store((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len > 1048576ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {
    MeshLibraryBytes invalid_response = {0};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (status != MESH_LIBRARY_OK) {
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}

JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_nonce(JNIEnv *env, jclass cls, jbyteArray request) {
  (void)cls;
  MeshLibraryBytes response = {0};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > 1048576ULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }
  uint8_t *request_data = NULL;
  if (request_len != 0) {
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }
  }
  int32_t status = authority_nonce((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len > 1048576ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {
    MeshLibraryBytes invalid_response = {0};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (status != MESH_LIBRARY_OK) {
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}

JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_actor_1start(JNIEnv *env, jclass cls, jbyteArray request) {
  (void)cls;
  MeshLibraryBytes response = {0};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > 1048576ULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }
  uint8_t *request_data = NULL;
  if (request_len != 0) {
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }
  }
  int32_t status = authority_actor((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len > 1048576ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {
    MeshLibraryBytes invalid_response = {0};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (status != MESH_LIBRARY_OK) {
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}

JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_private_1error(JNIEnv *env, jclass cls, jbyteArray request) {
  (void)cls;
  MeshLibraryBytes response = {0};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > 1048576ULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }
  uint8_t *request_data = NULL;
  if (request_len != 0) {
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }
  }
  int32_t status = authority_error((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len > 1048576ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {
    MeshLibraryBytes invalid_response = {0};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (status != MESH_LIBRARY_OK) {
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}

JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_timer_1start(JNIEnv *env, jclass cls, jbyteArray request) {
  (void)cls;
  MeshLibraryBytes response = {0};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > 1048576ULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }
  uint8_t *request_data = NULL;
  if (request_len != 0) {
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }
  }
  int32_t status = authority_timer((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len > 1048576ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {
    MeshLibraryBytes invalid_response = {0};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (status != MESH_LIBRARY_OK) {
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}

JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_terminate_1start(JNIEnv *env, jclass cls, jbyteArray request) {
  (void)cls;
  MeshLibraryBytes response = {0};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > 1048576ULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }
  uint8_t *request_data = NULL;
  if (request_len != 0) {
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }
  }
  int32_t status = authority_terminate((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len > 1048576ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {
    MeshLibraryBytes invalid_response = {0};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (status != MESH_LIBRARY_OK) {
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}

JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_gc(JNIEnv *env, jclass cls, jbyteArray request) {
  (void)cls;
  MeshLibraryBytes response = {0};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > 1048576ULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }
  uint8_t *request_data = NULL;
  if (request_len != 0) {
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }
  }
  int32_t status = authority_gc((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len > 1048576ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {
    MeshLibraryBytes invalid_response = {0};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (status != MESH_LIBRARY_OK) {
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}

JNIEXPORT jbyteArray JNICALL Java_mesh_MeshLibrary_unwind(JNIEnv *env, jclass cls, jbyteArray request) {
  (void)cls;
  MeshLibraryBytes response = {0};
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request == NULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_INVALID_ARGUMENT, &response);
    return NULL;
  }
  jsize request_len = (*env)->GetArrayLength(env, request);
  if ((*env)->ExceptionCheck(env)) return NULL;
  if (request_len < 0 || (uint64_t)request_len > 1048576ULL) {
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
    return NULL;
  }
  uint8_t *request_data = NULL;
  if (request_len != 0) {
    request_data = (uint8_t *)malloc((size_t)request_len);
    if (request_data == NULL) {
      mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &response);
      return NULL;
    }
    (*env)->GetByteArrayRegion(env, request, 0, request_len, (jbyte *)request_data);
    if ((*env)->ExceptionCheck(env)) {
      mesh_erase_and_free(request_data, (size_t)request_len);
      return NULL;
    }
  }
  int32_t status = authority_panic((const uint8_t *)request_data, (uint64_t)request_len, &response);
  mesh_erase_and_free(request_data, (size_t)request_len);
  if ((*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len > 1048576ULL || response.len > INT32_MAX || (response.len != 0 && response.data == NULL)) {
    MeshLibraryBytes invalid_response = {0};
    mesh_throw_library_failure(env, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE, &invalid_response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (status != MESH_LIBRARY_OK) {
    mesh_throw_library_failure(env, status, &response);
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  jbyteArray result = (*env)->NewByteArray(env, (jsize)response.len);
  if (result == NULL || (*env)->ExceptionCheck(env)) {
    mesh_library_free_returned_bytes(&response);
    return NULL;
  }
  if (response.len != 0) (*env)->SetByteArrayRegion(env, result, 0, (jsize)response.len, (const jbyte *)response.data);
  mesh_library_free_returned_bytes(&response);
  if ((*env)->ExceptionCheck(env)) return NULL;
  return result;
}
