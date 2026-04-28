# --- builder ---
# Use a recent stable image so it can read Cargo.lock v4 (introduced in 1.78).
FROM rust:1.82-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev libpq-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache deps
COPY Cargo.toml Cargo.lock ./
COPY migration/Cargo.toml ./migration/

# Source
COPY src ./src
COPY migration/src ./migration/src

RUN cargo build --release --bin api-crypto

# --- runtime ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libssl3 libpq5 curl \
    && rm -rf /var/lib/apt/lists/* \
    && update-ca-certificates

RUN groupadd -r app && useradd -r -g app app
WORKDIR /app

COPY --from=builder /app/target/release/api-crypto /app/api-crypto
RUN chmod +x /app/api-crypto && chown -R app:app /app

USER app
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -fsS http://localhost:8080/health || exit 1

CMD ["/app/api-crypto"]
