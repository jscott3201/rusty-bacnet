# Multi-stage Dockerfile for BACnet stress testing.
#
# Builder stage compiles all benchmark binaries.
# Runtime stage copies them into a minimal Alpine image.

# ── Builder ──────────────────────────────────────────────────────────────────
FROM rust:1.93-alpine AS builder

RUN apk add --no-cache musl-dev cmake make perl gcc g++

WORKDIR /src
COPY . .

# Build all benchmark binaries in release mode (static musl linking).
RUN cargo build --release -p bacnet-benchmarks \
    --bin stress-test \
    --bin bacnet-device \
    --bin bacnet-router \
    --bin bacnet-bbmd \
    --bin bacnet-sc-hub \
    --bin stress-orchestrator

# ── Runtime ──────────────────────────────────────────────────────────────────
FROM alpine:3.21 AS runtime

RUN apk add --no-cache ca-certificates

COPY --from=builder /src/target/release/stress-test       /usr/local/bin/
COPY --from=builder /src/target/release/bacnet-device     /usr/local/bin/
COPY --from=builder /src/target/release/bacnet-router     /usr/local/bin/
COPY --from=builder /src/target/release/bacnet-bbmd       /usr/local/bin/
COPY --from=builder /src/target/release/bacnet-sc-hub     /usr/local/bin/
COPY --from=builder /src/target/release/stress-orchestrator /usr/local/bin/

# Default command — override with docker-compose command.
CMD ["stress-test"]
