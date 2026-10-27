# CI supplies the same tested npm tarball used for package validation.
FROM docker.io/library/node:22.23.4-bookworm-slim AS install
WORKDIR /opt/hypervibes
COPY artifacts/hypervibes.tgz /tmp/hypervibes.tgz
RUN npm install --omit=dev --ignore-scripts --no-audit --no-fund /tmp/hypervibes.tgz \
    && rm /tmp/hypervibes.tgz

FROM docker.io/library/node:22.23.4-bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/idft/hypervibes" \
      org.opencontainers.image.licenses="AGPL-3.0-only"
ENV NODE_ENV=production \
    APP_HOST=0.0.0.0 \
    APP_PORT=3003 \
    XDG_CONFIG_HOME=/var/lib/hypervibes/config \
    XDG_DATA_HOME=/var/lib/hypervibes/data \
    APP_CACHE_DIR=/tmp/hypervibes-cache
WORKDIR /opt/hypervibes
COPY --from=install /opt/hypervibes /opt/hypervibes
RUN mkdir -p /var/lib/hypervibes/config /var/lib/hypervibes/data \
    && chown -R node:node /var/lib/hypervibes
USER node
VOLUME ["/var/lib/hypervibes"]
EXPOSE 3003
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["node", "-e", "const port=process.env.APP_PORT||'3003';const r=require('node:http').get({hostname:'127.0.0.1',port,path:'/healthz',headers:{host:new URL(process.env.APP_ORIGIN||'http://localhost:'+port).host},timeout:4000},s=>{s.resume();if(s.statusCode!==200)process.exit(1)});r.on('timeout',()=>r.destroy(new Error('timeout')));r.on('error',()=>process.exit(1))"]
# No shell or package manager between Docker and the signal handlers.
ENTRYPOINT ["node", "/opt/hypervibes/node_modules/hypervibes/bin/hypervibes.js"]
