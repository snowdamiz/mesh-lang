#define _DARWIN_C_SOURCE
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>

// Count-only interposer for DefaultStack's anonymous PROT_NONE reservation.
// Subsequent operations must match a tracked base and the exact guard offset.
struct region {
    void *base;
    size_t length;
};
struct counts {
    unsigned reserve, protect, reset, unmap, live, high;
};
static struct region regions[4096];
static struct counts small, actual;
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static size_t page;
static uint64_t owners[128];
static unsigned owner_count;

static struct counts *size_count(size_t total) {
    if (total == page + 64 * 1024) return &small;
    if (total == page + 512 * 1024) return &actual;
    return NULL;
}

static void remember_owner(void) {
    uint64_t id;
    if (pthread_threadid_np(NULL, &id) != 0) abort();
    for (unsigned i = 0; i < owner_count; ++i)
        if (owners[i] == id) return;
    if (owner_count == sizeof owners / sizeof *owners) abort();
    owners[owner_count++] = id;
}

static void change_payload(void *addr, size_t len, bool reset) {
    pthread_mutex_lock(&lock);
    for (size_t i = 0; i < sizeof regions / sizeof *regions; ++i) {
        if (regions[i].base != NULL &&
            (char *)regions[i].base + page == addr && regions[i].length == len + page) {
            struct counts *count = size_count(len + page);
            if (reset) ++count->reset;
            else ++count->protect;
            break;
        }
    }
    pthread_mutex_unlock(&lock);
}

static void *count_mmap(void *addr, size_t len, int prot, int flags, int fd, off_t offset) {
    void *base = mmap(addr, len, prot, flags, fd, offset);
    if (base == MAP_FAILED || page == 0) return base;
    if (prot == PROT_NONE && flags == (MAP_PRIVATE | MAP_ANON) && size_count(len)) {
        pthread_mutex_lock(&lock);
        size_t i = 0;
        while (i < sizeof regions / sizeof *regions && regions[i].base != NULL) ++i;
        if (i == sizeof regions / sizeof *regions) abort();
        regions[i] = (struct region){base, len};
        struct counts *count = size_count(len);
        ++count->reserve;
        if (++count->live > count->high) count->high = count->live;
        remember_owner();
        pthread_mutex_unlock(&lock);
    } else if ((flags & MAP_FIXED) && size_count(len + page) &&
               prot == (PROT_READ | PROT_WRITE)) {
        change_payload(base, len, true);
    }
    return base;
}

static int count_mprotect(void *addr, size_t len, int prot) {
    int status = mprotect(addr, len, prot);
    if (status == 0 && page && size_count(len + page) && prot == (PROT_READ | PROT_WRITE))
        change_payload(addr, len, false);
    return status;
}

static int count_munmap(void *addr, size_t len) {
    struct counts *count = page ? size_count(len) : NULL;
    if (count == NULL) return munmap(addr, len);
    pthread_mutex_lock(&lock);
    int status = munmap(addr, len);
    if (status == 0) {
        for (size_t i = 0; i < sizeof regions / sizeof *regions; ++i) {
            if (regions[i].base == addr && regions[i].length == len) {
                regions[i] = (struct region){0};
                ++count->unmap;
                --count->live;
                break;
            }
        }
    }
    pthread_mutex_unlock(&lock);
    return status;
}

#define INTERPOSE(new, old) \
    __attribute__((used, section("__DATA,__interpose"))) \
    static const struct { const void *replacement; const void *original; } interpose_##old = { \
        (const void *)(uintptr_t)&new, (const void *)(uintptr_t)&old \
    }
INTERPOSE(count_mmap, mmap);
INTERPOSE(count_mprotect, mprotect);
INTERPOSE(count_munmap, munmap);

__attribute__((constructor)) static void initialize(void) {
    page = (size_t)getpagesize();
}

__attribute__((destructor)) static void report(void) {
    if (small.reserve == 0 && actual.reserve == 0) return;
    fprintf(stderr, "STACK_CALLS {\"pid\":%d,\"page_bytes\":%zu,\"owners\":%u", getpid(), page, owner_count);
    const struct counts *all[] = {&small, &actual};
    const char *names[] = {"65536", "524288"};
    for (unsigned i = 0; i < 2; ++i) {
        const struct counts *c = all[i];
        fprintf(stderr, ",\"%s\":{\"reserve\":%u,\"protect\":%u,\"reset\":%u,"
                "\"unmap\":%u,\"live\":%u,\"peak_mappings\":%u}", names[i],
                c->reserve, c->protect, c->reset, c->unmap, c->live, c->high);
    }
    fputs("}\n", stderr);
}
