# Multi-stage Dockerfile for ctx-server
# Stage 1: Build ctx-server release binary with Rust 1.85 on Debian Bookworm
FROM rust:1.85-bookworm AS builder

WORKDIR /usr/src/ctx

# Copy full workspace (all crates and manifests)
COPY . .

# Build release binary for ctx-server
RUN cargo build --release --bin ctx-server

# Stage 2: Minimal runtime image based on Debian Bookworm Slim
FROM debian:bookworm-slim AS runtime

# Install TLS certificates for secure network calls (HTTPS / S3 / MinIO)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy ctx-server binary from builder stage
COPY --from=builder /usr/src/ctx/target/release/ctx-server /usr/local/bin/ctx-server

# Ensure binary is executable
RUN chmod +x /usr/local/bin/ctx-server

# Expose default HTTP service port
EXPOSE 9900

# Set entrypoint
ENTRYPOINT ["/usr/local/bin/ctx-server"]
