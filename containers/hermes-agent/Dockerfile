ARG HERMES_TAG=v2026.6.5
FROM docker.io/nousresearch/hermes-agent:${HERMES_TAG}

USER root

# System utilities.
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        jq \
        git && \
    rm -rf /var/lib/apt/lists/*

# Install Bun by copying from the official image. Use the fully-qualified
# registry path so podman does not issue an unqualified-search warning.
COPY --from=docker.io/oven/bun:1.3.14 /usr/local/bin/bun /usr/local/bin/bun

# Install the Vibetrading plugin's Python dependencies into the Hermes venv.
COPY hermes-plugin/vibetrading/requirements.txt /tmp/vibetrading-requirements.txt
RUN uv pip install --python /opt/hermes/.venv/bin/python -r /tmp/vibetrading-requirements.txt && \
    chown -R hermes:hermes /opt/hermes/.venv

# Patch the upstream config seed so the vibetrading plugin is enabled on first boot.
# This is YAML-aware and does not clobber existing example defaults.
RUN /opt/hermes/.venv/bin/python - <<'PY'
import yaml, pathlib
p = pathlib.Path('/opt/hermes/cli-config.yaml.example')
cfg = yaml.safe_load(p.read_text()) if p.exists() else {}
cfg.setdefault('plugins', {})
enabled = cfg['plugins'].setdefault('enabled', [])
if 'vibetrading' not in enabled:
    enabled.append('vibetrading')
p.write_text(yaml.safe_dump(cfg, default_flow_style=False, sort_keys=False))
PY

# Bake the plugin source into the image so a fresh named volume works standalone.
# Hermes scans user plugins at $HERMES_HOME/plugins/, and get_hermes_home()
# resolves to /opt/data in this container.
# In development this path is bind-mounted over from the repo.
COPY hermes-plugin/vibetrading /opt/data/plugins/vibetrading
RUN chown -R hermes:hermes /opt/data/plugins/vibetrading
