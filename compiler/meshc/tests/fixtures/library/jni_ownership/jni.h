#ifndef MESH_JNI_OWNERSHIP_FIXTURE_H
#define MESH_JNI_OWNERSHIP_FIXTURE_H
#include <stdint.h>
#define JNIEXPORT
#define JNICALL
#define JNI_ABORT 2
typedef int8_t jbyte;
typedef int32_t jint;
typedef int32_t jsize;
typedef uint8_t jboolean;
typedef void *jclass;
struct FixtureArray { jsize len; jbyte data[32]; };
typedef struct FixtureArray *jbyteArray;
struct JNINativeInterface_;
typedef const struct JNINativeInterface_ *JNIEnv;
/* Only the real C JNI signatures used by the generated wrapper are modelled.
 * This is a compiled ownership fixture, not a JVM ABI/layout emulator. */
struct JNINativeInterface_ {
    jsize (*GetArrayLength)(JNIEnv *, jbyteArray);
    jbyte *(*GetByteArrayElements)(JNIEnv *, jbyteArray, jboolean *);
    void (*ReleaseByteArrayElements)(JNIEnv *, jbyteArray, jbyte *, jint);
    void (*GetByteArrayRegion)(JNIEnv *, jbyteArray, jsize, jsize, jbyte *);
    jbyteArray (*NewByteArray)(JNIEnv *, jsize);
    void (*SetByteArrayRegion)(JNIEnv *, jbyteArray, jsize, jsize, const jbyte *);
    jclass (*FindClass)(JNIEnv *, const char *);
    jint (*ThrowNew)(JNIEnv *, jclass, const char *);
    jboolean (*ExceptionCheck)(JNIEnv *);
};
#endif
