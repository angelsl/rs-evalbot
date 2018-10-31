#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <seccomp.h>

int main() {
    scmp_filter_ctx ctx = seccomp_init(SCMP_ACT_KILL);
    if (!ctx) {
        fprintf(stderr, "seccomp_init failed\n");
        goto out;
    }

    char *scn = 0;
    int r = 0;
    while (scanf("%ms", &scn) != EOF) {
        int syscall_number = seccomp_syscall_resolve_name(scn);
        free(scn);
        if (syscall_number == __NR_SCMP_ERROR) {
            fprintf(stderr, "nonexistent syscall %s\n", scn);
            goto out;
        }
        if ((r = seccomp_rule_add(ctx, SCMP_ACT_ALLOW, syscall_number, 0)) < 0) {
            fprintf(stderr, "seccomp_rule_add failed: %s\n", scn, strerror(r));
            goto out;
        }
    }

    if ((r = seccomp_export_bpf(ctx, fileno(stdout))) < 0) {
        fprintf(stderr, "seccomp_export_bpf failed: %s\n", scn, strerror(r));
        goto out;
    }

    if (ctx) seccomp_release(ctx);
    return 0;
out:
    if (ctx) seccomp_release(ctx);
    return 1;
}
