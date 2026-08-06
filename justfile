test:
    cargo test --workspace
    python agent-runtime/test_coding_validate.py
    python agent-runtime/mcp/test_server.py
    pnpm test:js
