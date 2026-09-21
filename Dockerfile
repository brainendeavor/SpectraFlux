# syntax=docker/dockerfile:1
# ------------------------------------------------------------------------------
# SpectraFlux Runtime Dockerfile (High-Velocity Execution Chassis)
# ------------------------------------------------------------------------------

FROM rust:1-slim-bookworm AS builder
WORKDIR /usr/src/spectraflux

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    cmake \
    pkg-config \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY runtime ./runtime
COPY sdks ./sdks
COPY fluxcells ./fluxcells
COPY tools ./tools
COPY seeds ./seeds
COPY examples ./examples
COPY wit ./wit
COPY db ./db

RUN cargo build --release -p spectra-flux

# Ultra-lean runtime image (<30 MB)
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/spectraflux/target/release/spectra-flux /usr/local/bin/spectraflux
RUN ln -s /usr/local/bin/spectraflux /usr/local/bin/spectra-flux && \
    ln -s /usr/local/bin/spectraflux /usr/local/bin/spectral-flux

WORKDIR /etc/spectraflux
COPY spectra-flux.toml /etc/spectraflux/spectra-flux.toml
RUN ln -s /etc/spectraflux/spectra-flux.toml /etc/spectraflux/spectral-flux.toml

# Directory where .wasm fluxcells are mounted or dynamically staged
RUN mkdir -p /etc/spectraflux/fluxcells

EXPOSE 8081

ENV FLUX_PORT=8081
ENV FLUX_HOST=0.0.0.0
ENV FLUX_CONFIG=/etc/spectraflux/spectra-flux.toml

HEALTHCHECK --interval=5s --timeout=3s --retries=3 \
  CMD curl -f http://localhost:8081/healthz || exit 1

ENTRYPOINT ["spectraflux"]
