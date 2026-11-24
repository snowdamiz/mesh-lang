#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <poll.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>

enum { MAX_PIDS = 128, MAX_FRAMES = 4500, PERIOD_MS = 20, TICKS = 8 };
#define NS_PER_S UINT64_C(1000000000)

struct stamp { uint64_t before, epoch, after; };
struct read_cpu { uint64_t before, cpu, after; int error; };
struct frame {
    struct stamp begin, host_begin, host_end, end;
    uint64_t host[2][TICKS], self_cpu;
    struct read_cpu process[MAX_PIDS];
};
static struct frame frames[MAX_FRAMES];

static void fail(const char *what) {
    perror(what);
    exit(1);
}

static uint64_t ns(clockid_t clock) {
    struct timespec t;
    if (clock_gettime(clock, &t)) fail("clock_gettime");
    return (uint64_t)t.tv_sec * NS_PER_S + (uint64_t)t.tv_nsec;
}

static struct stamp stamp(void) {
    struct stamp s;
    s.before = ns(CLOCK_MONOTONIC);
    s.epoch = ns(CLOCK_REALTIME);
    s.after = ns(CLOCK_MONOTONIC);
    return s;
}

static long number(const char *value, long min, long max) {
    char *end;
    errno = 0;
    long n = strtol(value, &end, 10);
    if (errno || end == value || *end || n < min || n > max) {
        errno = EINVAL;
        fail("integer argument");
    }
    return n;
}

static void host_ticks(int fd, const int cpus[2], struct frame *f) {
    char buf[32768];
    f->host_begin = stamp();
    ssize_t size = pread(fd, buf, sizeof(buf) - 1, 0);
    f->host_end = stamp();
    if (size <= 0) fail("read /proc/stat");
    buf[size] = '\0';
    unsigned seen = 0;
    for (char *line = buf; *line; ++line) {
        char *end = line;
        while (*end && *end != '\n') ++end;
        if (*end != '\n') break;
        *end = '\0';
        int cpu;
        int used;
        if (sscanf(line, "cpu%d %n", &cpu, &used) == 1) {
            for (int i = 0; i < 2; ++i) {
                if (cpu != cpus[i]) continue;
                char *p = line + used;
                for (int j = 0; j < TICKS; ++j) {
                    char *next;
                    f->host[i][j] = strtoull(p, &next, 10);
                    if (next == p) {
                        errno = EIO;
                        fail("truncated host ticks");
                    }
                    p = next;
                }
                seen |= 1U << i;
            }
        }
        if (seen == 3) return;
        line = end;
    }
    errno = EIO;
    fail("missing workload CPU host ticks");
}

static void print_stamp(struct stamp s) {
    printf("[%" PRIu64 ",%" PRIu64 ",%" PRIu64 "]", s.before, s.epoch, s.after);
}

static void emit(const struct frame *f, int count) {
    printf("{\"begin\":");
    print_stamp(f->begin);
    printf(",\"host_begin\":");
    print_stamp(f->host_begin);
    printf(",\"host_end\":");
    print_stamp(f->host_end);
    printf(",\"end\":");
    print_stamp(f->end);
    printf(",\"ticks\":[");
    for (int i = 0; i < 2; ++i) {
        printf("%s[", i ? "," : "");
        for (int j = 0; j < TICKS; ++j)
            printf("%s%" PRIu64, j ? "," : "", f->host[i][j]);
        putchar(']');
    }
    printf("],\"self_cpu\":%" PRIu64 ",\"process\":[", f->self_cpu);
    for (int i = 0; i < count; ++i) {
        const struct read_cpu *r = &f->process[i];
        printf("%s[%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%d]",
               i ? "," : "", r->before, r->cpu, r->after, r->error);
    }
    puts("]}");
}

int main(int argc, char **argv) {
    if (argc < 6 || (argc - 4) % 2 || (argc - 4) / 2 > MAX_PIDS) {
        fputs("usage: clock CPU0 CPU1 SPARE PID PIDFD [PID PIDFD ...]\n", stderr);
        return 1;
    }
    int cpus[] = {number(argv[1], 0, CPU_SETSIZE - 1), number(argv[2], 0, CPU_SETSIZE - 1)};
    int spare = number(argv[3], 0, CPU_SETSIZE - 1);
    if (cpus[0] == cpus[1] || cpus[0] == spare || cpus[1] == spare) {
        errno = EINVAL;
        fail("observer and workload CPU masks overlap");
    }
    cpu_set_t mask;
    CPU_ZERO(&mask);
    CPU_SET(spare, &mask);
    if (sched_setaffinity(0, sizeof(mask), &mask)) fail("sampler affinity");
    int count = (argc - 4) / 2;
    struct pollfd watched[MAX_PIDS];
    clockid_t clocks[MAX_PIDS];
    int registration_errors[MAX_PIDS];
    for (int i = 0; i < count; ++i) {
        int pid = number(argv[4 + 2*i], 1, INT32_MAX);
        int fd = number(argv[5 + 2*i], 3, INT32_MAX);
        watched[i] = (struct pollfd){.fd = fd, .events = POLLIN};
        registration_errors[i] = clock_getcpuclockid(pid, &clocks[i]);
    }
    int host_fd = open("/proc/stat", O_RDONLY | O_CLOEXEC);
    if (host_fd < 0) fail("open /proc/stat");

    struct pollfd stop = {.fd = STDIN_FILENO, .events = POLLIN};
    int used = 0;
    uint64_t previous_start = 0;
    for (; used < MAX_FRAMES; ++used) {
        struct frame *f = &frames[used];
        f->begin = stamp();
        host_ticks(host_fd, cpus, f);
        for (int i = 0; i < count; ++i) {
            struct read_cpu *r = &f->process[i];
            r->before = ns(CLOCK_MONOTONIC);
            r->error = registration_errors[i];
            if (!r->error) {
                struct timespec t;
                if (clock_gettime(clocks[i], &t)) r->error = errno;
                else r->cpu = (uint64_t)t.tv_sec * NS_PER_S + (uint64_t)t.tv_nsec;
            }
            r->after = ns(CLOCK_MONOTONIC);
        }
        // Every successful read precedes this check of the original lifetime.
        // An exited process stays readable through the pidfd even after reuse.
        if (poll(watched, count, 0) < 0) fail("poll pidfds");
        for (int i = 0; i < count; ++i) {
            if (watched[i].revents) {
                f->process[i].error = (watched[i].revents & POLLNVAL) ? EBADF : ESRCH;
                f->process[i].cpu = 0;
            }
        }
        f->end = stamp();
        f->self_cpu = ns(CLOCK_PROCESS_CPUTIME_ID);
        if (!used) {
            fputs("ready\n", stderr);
            fflush(stderr);
        }
        if (f->begin.before <= previous_start) {
            errno = EIO;
            fail("monotonic frame order");
        }
        previous_start = f->begin.before;
        uint64_t deadline = previous_start + PERIOD_MS * UINT64_C(1000000);
        for (;;) {
            uint64_t now = ns(CLOCK_MONOTONIC);
            int ms = now < deadline ? (int)((deadline - now + 999999) / 1000000) : 0;
            int result = poll(&stop, 1, ms);
            if (result > 0) goto done;
            if (result == 0 || errno != EINTR) {
                if (result < 0) fail("poll stop pipe");
                break;
            }
        }
    }
done:
    for (int i = 0; i <= used && i < MAX_FRAMES; ++i) emit(&frames[i], count);
    if (fflush(stdout) || ferror(stdout)) fail("write trace");
    if (used == MAX_FRAMES) {
        errno = EOVERFLOW;
        fail("trace limit");
    }
    return 0;
}
