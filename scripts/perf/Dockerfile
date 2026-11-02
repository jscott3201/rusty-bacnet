# Callgrind image for the counting harness (benchmarks/counts/,
# scripts/perf/perf_counts.py, Benchmarks.md "Counting harness"). The script
# builds it on demand and tags it rb-perf-counts:<first 12 hex digits of
# this file's SHA-256>, so an edit here gives a new tag; it never pulls one.
#
# arm64 only (the count archive is arm64: Apple silicon Docker locally,
# GitHub's ubuntu-24.04-arm runner in .github/workflows/perf-counts.yml).
# Counts compare only within one image on one machine: glibc picks its
# memcpy variant by CPU.
#
# Reproducible inputs:
# - Ubuntu 24.04 pinned by its index digest (the 2026-10-24 build).
# - rustup-init at the version and SHA-256 .github/ci-pins.env pins
#   (RUSTUP_VERSION, RUSTUP_AARCH64_SHA256), passed in as build arguments,
#   then the toolchain rust-toolchain.toml pins (RUST_TOOLCHAIN).
# - Valgrind and the C toolchain from apt at the versions in Ubuntu 24.04's
#   release pocket, which never changes. VALGRIND_VERSION is the one #2012's
#   counts used (3.22.0).
#
# Relation to rb-callgrind:local: the earlier counts (#1713, #1978, PR
# #2012) ran in a local-only image with no committed recipe (ID 0f6ec719,
# built 2026-10-18), plus a one-line derivative that installed Rust 1.99.0.
# Its history shows the same base (Ubuntu 24.04) and apt valgrind, with
# rustup's latest installer, Rust 1.97.1 and extra packages the harness
# doesn't need (libudev-dev, cmake, git, pkg-config: aws-lc and serial).
# This file replaces both; counts taken in the two images aren't
# compared with each other.
FROM ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852

ARG RUSTUP_VERSION
ARG RUSTUP_AARCH64_SHA256
ARG RUST_TOOLCHAIN
ARG VALGRIND_VERSION=1:3.22.0-0ubuntu3

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    CARGO_TERM_COLOR=never \
    CARGO_INCREMENTAL=0

RUN test "$(dpkg --print-architecture)" = arm64 \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl perl python3 \
      "valgrind=${VALGRIND_VERSION}" \
    && rm -rf /var/lib/apt/lists/*

RUN test -n "$RUSTUP_VERSION" && test -n "$RUSTUP_AARCH64_SHA256" && test -n "$RUST_TOOLCHAIN" \
    && curl --proto '=https' --tlsv1.2 -sSfL -o /tmp/rustup-init \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/aarch64-unknown-linux-gnu/rustup-init" \
    && echo "${RUSTUP_AARCH64_SHA256}  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain "$RUST_TOOLCHAIN" \
    && rm /tmp/rustup-init \
    && rustc -vV && valgrind --version

# Both trees build with this toolchain, whatever their rust-toolchain.toml
# says, so a baseline and a candidate differ only in source.
ENV RUSTUP_TOOLCHAIN=${RUST_TOOLCHAIN}

WORKDIR /wt
