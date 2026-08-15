#!/bin/sh
set -eu

role="${1:-}"

case "$role" in
    hypervibes)
        shift
        exec hypervibes "$@"
        ;;
    opencode)
        shift
        workspace-controller &
        controller_pid=$!

        opencode "$@" &
        opencode_pid=$!

        terminate() {
            kill -TERM "$controller_pid" "$opencode_pid" 2>/dev/null || true
        }

        trap terminate INT TERM

        wait -n "$controller_pid" "$opencode_pid"
        status=$?
        terminate
        wait "$controller_pid" 2>/dev/null || true
        wait "$opencode_pid" 2>/dev/null || true
        exit "$status"
        ;;
    *)
        echo "usage: hypervibes-entrypoint {hypervibes|opencode}" >&2
        exit 64
        ;;
esac
