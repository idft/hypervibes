test:
    cargo test --workspace
    python agent-runtime/test_coding_validate.py
    python agent-runtime/mcp/test_server.py
    pnpm test:js

dev:
    podman-compose -f podman-compose.dev.yaml up

website:
    pnpm --dir website start --host 0.0.0.0
