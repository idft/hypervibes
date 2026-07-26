#!/bin/sh
set -eu

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
