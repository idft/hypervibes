ARG OPENCODE_TAG=1.18.7

FROM docker.io/library/rust:1.96-alpine AS workspace-controller-builder

RUN apk add --no-cache musl-dev
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY crates/workspace-store crates/workspace-store
COPY crates/workspace-controller crates/workspace-controller
RUN cargo build --release --package workspace-controller

FROM ghcr.io/anomalyco/opencode:${OPENCODE_TAG}

# `uv` is a single static binary copied from the official image so the
# Dockerfile does not have to bootstrap a Python package manager in
# Alpine. It is used below to create separate analysis and MCP virtualenvs
# and install their Python dependencies.
COPY --from=docker.io/astral/uv:0.10-alpine /usr/local/bin/uv /usr/local/bin/uvx /usr/local/bin/

USER root

# The upstream OpenCode image is Alpine-based and ships neither Python nor
# pip. Install Python 3 for simple runtime health checks, then use a
# uv-managed CPython for dependency-heavy virtualenvs so Alpine package updates
# do not silently move the analysis stack to a Python without wheels.
RUN apk add --no-cache \
        ca-certificates \
        libgomp \
        libstdc++ \
        python3

RUN uv python install 3.13

# Bake the container-global OpenCode config into the image. Compose points
# OPENCODE_CONFIG at this path while /opencode-data remains mutable state.
COPY agent-runtime/container/opencode.jsonc /opt/vibetrading/opencode/opencode.jsonc
COPY --from=workspace-controller-builder /build/target/release/workspace-controller /usr/local/bin/workspace-controller
COPY agent-runtime/workspace-template /opt/vibetrading/workspace-template
COPY containers/opencode/entrypoint.sh /usr/local/bin/vibetrading-opencode
RUN chmod +x /usr/local/bin/vibetrading-opencode

# Shared Python analysis runtime for generated agent workspaces. Keep this
# separate from the MCP server venv so analysis dependencies do not affect MCP.
COPY agent-runtime/analysis/requirements.txt /opt/vibetrading/analysis/requirements.txt
RUN uv venv --python 3.13 /opt/vibetrading/analysis/.venv \
    && uv pip install --python /opt/vibetrading/analysis/.venv/bin/python \
        --no-cache \
        --requirement /opt/vibetrading/analysis/requirements.txt \
    && /opt/vibetrading/analysis/.venv/bin/pyright --version

ENV PATH="/opt/vibetrading/analysis/.venv/bin:${PATH}"

# Fixed coding MCP validator. Candidate reports may request validation,
# but the model cannot select a different executable or command.
COPY agent-runtime/coding_validate.py /opt/vibetrading/coding/coding_validate.py
COPY agent-runtime/coding_fixture.json /opt/vibetrading/coding/coding_fixture.json

# Bake the Vibetrading MCP server source into a fixed image path so the
# generated per-workspace `opencode.json` can launch it directly without
# relying on shell startup files. `uv` creates the venv and installs the
# pinned Python dependencies from `requirements.txt`.
COPY agent-runtime/mcp /opt/vibetrading/mcp
RUN uv venv --python 3.13 /opt/vibetrading/mcp/.venv \
    && uv pip install --python /opt/vibetrading/mcp/.venv/bin/python \
        --no-cache \
        --requirement /opt/vibetrading/mcp/requirements.txt

ENTRYPOINT ["/usr/local/bin/vibetrading-opencode"]
