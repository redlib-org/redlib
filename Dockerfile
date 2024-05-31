FROM docker.io/library/rust:1.78-slim-bookworm AS builder

WORKDIR /app
COPY ./ ./

RUN cargo test --release
RUN cargo build --release

FROM docker.io/library/debian:bookworm-slim AS release

WORKDIR /app
# ca-certificates are not preinstalled in the base image
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder --chown=600 /app/target/release/redlib /app/

RUN useradd -M redlib
USER redlib

# Tell Docker to expose port 8080
EXPOSE 8080

# Run a healthcheck every minute to make sure redlib is functional
HEALTHCHECK --interval=1m --timeout=3s CMD wget --spider --q http://localhost:8080/settings || exit 1

CMD ["/app/redlib"]

