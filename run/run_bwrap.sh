#!/usr/bin/bash
bpffile="$1"
timeout="$2"
shift 2
exec timeout -s 9 "$timeout" bwrap --unshare-pid --die-with-parent --seccomp 63 --bind /opt/sandbox / --proc /proc --tmpfs /tmp --tmpfs /home/eval --chdir /home/eval "$@" 63<"$bpffile"
