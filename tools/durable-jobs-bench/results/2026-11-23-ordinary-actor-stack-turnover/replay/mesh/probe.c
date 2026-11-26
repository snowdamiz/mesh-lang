#define _DARWIN_C_SOURCE
#include <libproc.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <time.h>
#include <unistd.h>

enum { MAX_ACTORS = 1 << 20 };
static unsigned char stages[MAX_ACTORS];
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t progress = PTHREAD_COND_INITIALIZER;
static int64_t ready, finished, target, watching_finished, pairs;
static struct rusage begin;
static struct timespec started;

int64_t actor_probe_ready(int64_t ticket) {
    pthread_mutex_lock(&lock);
    if (ticket < 0 || ticket >= MAX_ACTORS || stages[ticket] != 0) abort();
    stages[ticket] = 1;
    if (++ready == target && !watching_finished) pthread_cond_signal(&progress);
    pthread_mutex_unlock(&lock);
    return 0;
}

int64_t actor_probe_finish(int64_t ticket, int64_t received) {
    pthread_mutex_lock(&lock);
    if (ticket < 0 || ticket >= MAX_ACTORS || ticket != received || stages[ticket] == 2) abort();
    pairs += stages[ticket] == 1;
    stages[ticket] = 2;
    if (++finished == target && watching_finished) pthread_cond_signal(&progress);
    pthread_mutex_unlock(&lock);
    return 0;
}

int64_t actor_probe_await(int64_t total, int64_t is_finished) {
    pthread_mutex_lock(&lock);
    int64_t *count = is_finished ? &finished : &ready;
    target = total;
    watching_finished = is_finished;
    while (*count < total) pthread_cond_wait(&progress, &lock);
    if (*count != total) abort();
    pthread_mutex_unlock(&lock);
    return 0;
}

static double elapsed(struct timeval v) {
    return (double)v.tv_sec + v.tv_usec / 1e6;
}

int64_t actor_probe_checkpoint(int64_t phase) {
    if (phase == 0) {
        if (getrusage(RUSAGE_SELF, &begin) != 0 || clock_gettime(CLOCK_MONOTONIC, &started) != 0)
            abort();
        if (!getenv("PROBE_MEMORY") || atoi(getenv("PROBE_MEMORY")) == 0) return 0;
    }

    // Resource-only schedule: let workers settle before sampling. No pause in
    // timed runs. A live cohort cannot return until the main actor sends.
    struct timespec pause = {0, 40000000};
    if (nanosleep(&pause, NULL) != 0) abort();
    struct proc_taskinfo task;
    if (proc_pidinfo(getpid(), PROC_PIDTASKINFO, 0, &task, sizeof task) != sizeof task) abort();
    printf("PHASE {\"id\":%lld,\"ready\":%lld,\"finished\":%lld,\"rss\":%llu,"
           "\"vm\":%llu,\"threads\":%d}\n", (long long)phase, (long long)ready,
           (long long)finished, (unsigned long long)task.pti_resident_size,
           (unsigned long long)task.pti_virtual_size, task.pti_threadnum);
    return 0;
}

int64_t actor_probe_report(int64_t total, int64_t held) {
    struct timespec stopped;
    struct rusage end;
    if (getrusage(RUSAGE_SELF, &end) != 0 || clock_gettime(CLOCK_MONOTONIC, &stopped) != 0)
        abort();
    if (finished != total || ready != held * total || pairs != held * total) abort();
    for (int64_t i = 0; i < total; ++i)
        if (stages[i] != 2) abort();
    printf("WORK {\"completed\":%lld,\"ready\":%lld,\"body_wall_s\":%.9f,"
           "\"body_user_s\":%.6f,\"body_system_s\":%.6f,\"body_minor_faults\":%ld}\n",
           (long long)finished, (long long)ready,
           (double)(stopped.tv_sec - started.tv_sec) + (stopped.tv_nsec - started.tv_nsec) / 1e9,
           elapsed(end.ru_utime) - elapsed(begin.ru_utime), elapsed(end.ru_stime) - elapsed(begin.ru_stime),
           end.ru_minflt - begin.ru_minflt);
    return 0;
}
