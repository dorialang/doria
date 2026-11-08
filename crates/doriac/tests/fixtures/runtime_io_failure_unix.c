#if defined(__linux__) && !defined(_GNU_SOURCE)
#define _GNU_SOURCE
#endif

#include <errno.h>
#include <stddef.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#if defined(__APPLE__)
#include <sys/event.h>
#elif defined(__linux__)
#include <dlfcn.h>
#include <unistd.h>
#else
#error Unsupported failure-injection platform
#endif

static atomic_flag failed = ATOMIC_FLAG_INIT;

static int enabled(const char *mode)
{
    const char *selected = getenv("DORIA_TEST_IO_FAILURE");
    return selected != NULL && strcmp(selected, mode) == 0;
}

#if defined(__APPLE__)
static int fail_one_notification(int queue, const struct kevent *changes,
                                 int change_count, struct kevent *events,
                                 int event_count, const struct timespec *timeout)
{
    if (enabled("cancellation") && change_count == 1 && changes != NULL &&
        changes[0].filter == EVFILT_USER &&
        (changes[0].fflags & NOTE_TRIGGER) != 0 &&
        !atomic_flag_test_and_set(&failed)) {
        errno = EIO;
        return -1;
    }
    return kevent(queue, changes, change_count, events, event_count, timeout);
}

static int fail_monotonic_clock(clockid_t clock_id, struct timespec *value)
{
    if (enabled("clock") && clock_id == CLOCK_MONOTONIC) {
        errno = EIO;
        return -1;
    }
    return clock_gettime(clock_id, value);
}

/* dyld excludes this image from its own replacement, preserving real calls:
 * https://github.com/apple-oss-distributions/dyld/blob/main/dyld/DyldRuntimeState.h
 */
__attribute__((used, section("__DATA,__interpose")))
static const struct {
    const void *replacement;
    const void *original;
} runtime_failures[] = {
    {(const void *)&fail_one_notification, (const void *)&kevent},
    {(const void *)&fail_monotonic_clock, (const void *)&clock_gettime}
};
#elif defined(__linux__)
#if defined(RTLD_DEEPBIND)
/* PHP before 8.5 loads extensions and FFI libraries with RTLD_DEEPBIND, which
 * binds their imports to libc ahead of this library. Binding them normally
 * keeps these replacements in front:
 * https://github.com/php/php-src/blob/PHP-8.4/Zend/zend_portability.h
 */
void *dlopen(const char *path, int flags)
{
    void *(*original)(const char *, int) = dlsym(RTLD_NEXT, "dlopen");
    if (original == NULL) { abort(); }
    return original(path, flags & ~RTLD_DEEPBIND);
}
#endif

ssize_t write(int descriptor, const void *bytes, size_t length)
{
    uint64_t value = 0;
    if (enabled("cancellation") && bytes != NULL && length == sizeof(value)) {
        memcpy(&value, bytes, sizeof(value));
        if (value == 1 && !atomic_flag_test_and_set(&failed)) {
            errno = EIO;
            return -1;
        }
    }
    ssize_t (*original)(int, const void *, size_t) = dlsym(RTLD_NEXT, "write");
    if (original == NULL) { abort(); }
    return original(descriptor, bytes, length);
}

int clock_gettime(clockid_t clock_id, struct timespec *value)
{
    if (enabled("clock") && clock_id == CLOCK_MONOTONIC) {
        errno = EIO;
        return -1;
    }
    int (*original)(clockid_t, struct timespec *) = dlsym(RTLD_NEXT, "clock_gettime");
    if (original == NULL) { abort(); }
    return original(clock_id, value);
}
#endif
