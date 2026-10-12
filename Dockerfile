# syntax=docker/dockerfile:1
FROM node:22-alpine AS web
WORKDIR /app/web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

FROM rust:1-alpine AS server
RUN apk add --no-cache musl-dev build-base
WORKDIR /app
COPY server/ server/
COPY --from=web /app/web/dist web/dist
WORKDIR /app/server
RUN cargo build --release --locked

FROM scratch
COPY --from=server /app/server/target/release/cratarr-server /cratarr-server
ENV CRATARR_BIND=0.0.0.0:8080
USER 1010:1001
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --retries=3 CMD ["/cratarr-server", "--healthcheck"]
ENTRYPOINT ["/cratarr-server"]
