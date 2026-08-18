ARG OPENCODE_TAG=1.18.18

FROM docker.io/library/rust:1.96-alpine AS builder

RUN apk add --no-cache \
        build-base \
        musl-dev \
        nodejs \
        npm \
    && npm install --global pnpm@11.3.0

WORKDIR /build

COPY Cargo.toml Cargo.lock build.rs ./
COPY src src
COPY crates crates
COPY migrations migrations
COPY templates templates
COPY assets assets
COPY static static
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./

# build.rs installs the locked frontend dependencies and produces static/dist.
RUN cargo build --release --package hypervibes --package workspace-controller

FROM ghcr.io/anomalyco/opencode:${OPENCODE_TAG}

ARG VERSION=dev

LABEL org.opencontainers.image.source="https://github.com/johnkozan/hypervibes" \
      org.opencontainers.image.version="${VERSION}"

# `uv` is a static binary copied from its Alpine image. It installs the
# isolated Python runtimes used by the analysis and MCP tools below.
COPY --from=docker.io/astral/uv:0.10-alpine /usr/local/bin/uv /usr/local/bin/uvx /usr/local/bin/

USER root

RUN apk add --no-cache \
        ca-certificates \
        freetype \
        libgomp \
        libpng \
        libstdc++ \
        python3 \
        zlib \
    && apk add --no-cache --virtual .python-build-deps \
        build-base \
        clang \
        compiler-rt \
        freetype-dev \
        libpng-dev \
        llvm22 \
        pkgconf \
        zlib-dev

RUN uv python install 3.13

COPY agent-runtime/container/opencode.jsonc /opt/hypervibes/opencode/opencode.jsonc
COPY agent-runtime/workspace-template /opt/hypervibes/workspace-template
COPY agent-runtime/analysis/requirements.txt /opt/hypervibes/analysis/requirements.txt
COPY agent-runtime/coding_validate.py /opt/hypervibes/coding/coding_validate.py
COPY agent-runtime/coding_fixture.json /opt/hypervibes/coding/coding_fixture.json
COPY agent-runtime/mcp /opt/hypervibes/mcp
COPY --from=builder /build/target/release/hypervibes /usr/local/bin/hypervibes
COPY --from=builder /build/target/release/workspace-controller /usr/local/bin/workspace-controller
COPY --from=builder /build/static /opt/hypervibes/static
COPY containers/entrypoint.sh /usr/local/bin/hypervibes-entrypoint

RUN chmod +x /usr/local/bin/hypervibes-entrypoint \
    && uv venv --python 3.13 /opt/hypervibes/analysis/.venv \
    && AR=llvm22-ar CC=clang CXX=clang++ RANLIB=llvm22-ranlib uv pip install --python /opt/hypervibes/analysis/.venv/bin/python \
        --no-cache \
        --requirement /opt/hypervibes/analysis/requirements.txt \
    && /opt/hypervibes/analysis/.venv/bin/pyright --version \
    && uv venv --python 3.13 /opt/hypervibes/mcp/.venv \
    && uv pip install --python /opt/hypervibes/mcp/.venv/bin/python \
        --no-cache \
        --requirement /opt/hypervibes/mcp/requirements.txt \
    && /opt/hypervibes/mcp/.venv/bin/python -c 'from mcp.server.fastmcp import FastMCP' \
    && apk del .python-build-deps

ENV PATH="/opt/hypervibes/analysis/.venv/bin:${PATH}"

WORKDIR /opt/hypervibes

EXPOSE 3003 14096 14097

ENTRYPOINT ["/usr/local/bin/hypervibes-entrypoint"]
