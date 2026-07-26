ARG VARIANT="trixie"

# ---- Dev stage (used by devcontainer) ----
FROM mcr.microsoft.com/devcontainers/rust:2-1-${VARIANT} AS dev
# Add any dev-only system packages here if needed

# ---- Builder stage (compiles release binary) ----
FROM mcr.microsoft.com/devcontainers/rust:2-1-${VARIANT} AS builder
WORKDIR /build
COPY . .
RUN cargo build --release

# ---- Runtime stage (for deployment) ----
FROM debian:${VARIANT}-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /bin/false blog
WORKDIR /app
COPY --from=builder /build/target/release/web /app/web
COPY --from=builder /build/content /app/content
COPY --from=builder /build/static /app/static
ENV CONTENT_DIR=/app/content \
    STATIC_DIR=/app/static
USER blog
EXPOSE 3000
CMD ["/app/web"]
