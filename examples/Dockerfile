# Flovyn SDK Examples - Multi-binary Docker image
#
# Build: docker build -t flovyn-examples -f examples/Dockerfile .
# Run:   docker run -e EXAMPLE=hello-world -e FLOVYN_WORKER_TOKEN=... flovyn-examples
#
# Available examples:
#   - hello-world
#   - ecommerce
#   - data-pipeline
#   - patterns
#   - standalone-tasks

# Build stage
FROM rust:1.82-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY worker-core ./worker-core
COPY worker-sdk ./worker-sdk
COPY worker-ffi ./worker-ffi
COPY examples ./examples

# Build all examples in release mode
RUN cargo build --release \
    -p hello-world-sample \
    -p ecommerce-sample \
    -p data-pipeline-sample \
    -p patterns-sample \
    -p standalone-tasks-sample

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy all example binaries
COPY --from=builder /app/target/release/hello-world-sample /app/bin/hello-world
COPY --from=builder /app/target/release/ecommerce-sample /app/bin/ecommerce
COPY --from=builder /app/target/release/data-pipeline-sample /app/bin/data-pipeline
COPY --from=builder /app/target/release/patterns-sample /app/bin/patterns
COPY --from=builder /app/target/release/standalone-tasks-sample /app/bin/standalone-tasks

# Copy entrypoint script
COPY examples/docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/docker-entrypoint.sh

# Environment variables (can be overridden at runtime)
ENV EXAMPLE=hello-world
ENV FLOVYN_GRPC_SERVER_URL=http://localhost:9090
ENV FLOVYN_WORKER_TOKEN=""
ENV FLOVYN_ORG_ID=""
ENV FLOVYN_QUEUE=default
ENV RUST_LOG=info

ENTRYPOINT ["/app/docker-entrypoint.sh"]
