#!/bin/sh
set -eu

PLUGIN_DIR="/opt/data/plugins/vibetrading"
VENV_DIR="$PLUGIN_DIR/.venv"
REQS_FILE="$PLUGIN_DIR/requirements.txt"
STAMP_FILE="$VENV_DIR/.requirements-installed"

if [ ! -f "$REQS_FILE" ]; then
    exit 0
fi

if [ ! -x "$VENV_DIR/bin/python" ] || [ ! -f "$STAMP_FILE" ] || [ "$REQS_FILE" -nt "$STAMP_FILE" ]; then
    uv venv "$VENV_DIR"
    uv pip install --python "$VENV_DIR/bin/python" -r "$REQS_FILE"
    touch "$STAMP_FILE"
fi

chown -R hermes:hermes "$PLUGIN_DIR"
