#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <stddef.h>
#include <stdarg.h>
#include <stdlib.h>

#include <unistd.h>
#include <errno.h>
#include <err.h>
#include <sys/socket.h>
#include <sys/un.h>

#define SOCKETPATH "test.sock"

static const int ONE = 1;

__attribute__((format(printf, 2, 3))) static void check_posix(intmax_t rc, const char *fmt, ...) {
    va_list args;
    va_start(args, fmt);
    if (rc == -1) verr(EXIT_FAILURE, fmt, args);
    va_end(args);
}

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <path to daemon> [args...]\n", argv[0]);
        return 1;
    }
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un addr = { .sun_family = AF_UNIX };
    strncpy(addr.sun_path, SOCKETPATH, sizeof(addr.sun_path) - 1);
    check_posix(fd, "socket");
    check_posix(setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &ONE, sizeof(ONE)), "setsockopt");
    check_posix(bind(fd, (struct sockaddr *) &addr, sizeof(addr)), "bind");
    if (fd != 3) {
        check_posix(dup2(fd, 3), "dup2");
        check_posix(close(fd), "close");
    }
    check_posix(execv(argv[1], argv + 1), "execv");
    return 1;
}
