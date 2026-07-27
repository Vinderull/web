ARG VARIANT="trixie"
ARG MUSL_TARGET="x86_64-unknown-linux-musl"

# ---- Dev stage (used by devcontainer) ----
FROM mcr.microsoft.com/devcontainers/rust:2-1-${VARIANT} AS dev
ARG MUSL_TARGET
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add ${MUSL_TARGET}

# ---- Builder stage (compiles release binary) ----
FROM mcr.microsoft.com/devcontainers/rust:2-1-${VARIANT} AS builder
ARG MUSL_TARGET
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add ${MUSL_TARGET}
WORKDIR /build
COPY . .
RUN cargo build --release --target ${MUSL_TARGET}

# ---- Runtime stage (for deployment) ----
# Distroless static: no shell, no package manager, no libc (binary is musl-static).
# :nonroot runs as UID 65532 without needing adduser.
FROM gcr.io/distroless/static-debian13:nonroot AS runtime
ARG MUSL_TARGET
WORKDIR /app
COPY --from=builder /build/target/${MUSL_TARGET}/release/web /app/web
COPY --from=builder /build/content /app/content
COPY --from=builder /build/static /app/static
ENV CONTENT_DIR=/app/content \
    STATIC_DIR=/app/static
USER nonroot:nonroot
EXPOSE 3000
CMD ["/app/web"]
