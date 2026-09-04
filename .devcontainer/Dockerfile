ARG VARIANT="trixie"
ARG MUSL_TARGET="x86_64-unknown-linux-musl"

# ---- Dev stage (devcontainer base; toolchain source for builder) ----
FROM mcr.microsoft.com/devcontainers/rust:${VARIANT} AS dev
ARG MUSL_TARGET
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add ${MUSL_TARGET}

# ---- Builder stage (compiles release binary) ----
# Reuses the dev stage so the CI runtime build and the devcontainer share one
# toolchain bootstrap (same base image, musl-tools, and target registration).
FROM dev AS builder
ARG MUSL_TARGET
WORKDIR /build
COPY . .
# Buildkit cache mounts persist the cargo registry + target dir across builds,
# so redeploys only recompile changed crates instead of the full tree.
# The target dir is a cache mount (ephemeral, not in the image layer), so the
# binary is copied out of it within the same RUN; a later COPY can't see it.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked --target ${MUSL_TARGET} \
    && cp target/${MUSL_TARGET}/release/web /web

# ---- Runtime stage (for deployment) ----
# Scratch: the binary is fully static musl, zero deps. No shell, no
# package manager, no libc, no /etc/ssl/certs, no /etc/passwd.
FROM scratch AS runtime
WORKDIR /app
COPY --from=builder /web /app/web
COPY --from=builder /build/content /app/content
ENV CONTENT_DIR=/app/content
USER 65532:65532
EXPOSE 3000
CMD ["/app/web"]
