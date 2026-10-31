# Loki server image: the web app plus loki-server and its command-line tools. Build with `docker build -t loki .` (or use compose.yaml).

FROM node:22-slim AS web
WORKDIR /src
COPY package.json package-lock.json ./
RUN npm ci
COPY index.html tsconfig.json tsconfig.node.json vite.config.ts ./
COPY public ./public
COPY src ./src
RUN npm run build

FROM rust:1-bookworm AS server
RUN apt-get update && apt-get install -y --no-install-recommends \
      libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY src-tauri ./src-tauri
COPY packs ./packs
COPY --from=web /src/dist ./dist
WORKDIR /src/src-tauri
RUN cargo build --release --bin loki-server --bin loki-cli --bin loki-mcp --bin loki-bridge-mcp

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --home-dir /data --uid 10001 loki
COPY --from=server /src/src-tauri/target/release/loki-server /src/src-tauri/target/release/loki-cli /src/src-tauri/target/release/loki-mcp /src/src-tauri/target/release/loki-bridge-mcp /usr/local/bin/
COPY --from=web /src/dist /app/dist
COPY LICENSE NOTICE /app/
# Everything Loki saves (config, login, notes cache) lives under /data, which compose mounts as a volume.
ENV HOME=/data LOKI_STATIC_DIR=/app/dist LOKI_BIND=0.0.0.0 LOKI_PORT=8420 LOKI_IN_CONTAINER=1 LOKI_SETUP_ALLOW_REMOTE=1
USER loki
WORKDIR /data
VOLUME /data
EXPOSE 8420
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s CMD curl -fs http://127.0.0.1:8420/ -o /dev/null
CMD ["loki-server"]
