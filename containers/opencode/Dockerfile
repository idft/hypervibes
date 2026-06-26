ARG OPENCODE_TAG=1.17.11
FROM ghcr.io/anomalyco/opencode:${OPENCODE_TAG}

# `uv` is a single static binary copied from the official image so the
# Dockerfile does not have to bootstrap a Python package manager in
# Alpine. It is used below to create the MCP server virtualenv and to
# install its Python dependencies.
COPY --from=docker.io/astral/uv:0.10-alpine /usr/local/bin/uv /usr/local/bin/uvx /usr/local/bin/

USER root

# The upstream OpenCode image is Alpine-based and ships neither Python
# nor pip. Install Python 3 with `apk`; the MCP virtualenv is created
# with `uv` and the system Python is only used to seed it.
RUN apk add --no-cache \
        ca-certificates \
        python3

# Bake the Vibetrading MCP server source into a fixed image path so the
# generated per-workspace `opencode.json` can launch it directly without
# relying on shell startup files. `uv` creates the venv and installs the
# pinned Python dependencies from `requirements.txt`.
COPY agent-runtime/opencode/mcp /opt/vibetrading/mcp
RUN uv venv /opt/vibetrading/mcp/.venv \
    && uv pip install --python /opt/vibetrading/mcp/.venv/bin/python \
        --no-cache \
        --requirement /opt/vibetrading/mcp/requirements.txt
