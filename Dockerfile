FROM rust:1.85-alpine AS builder
RUN apk add --no-cache musl-dev musl-dev openssl-dev perl make gcc
WORKDIR /app
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

FROM alpine:3.19

RUN apk add --no-cache curl openssl

COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/redlib /usr/local/bin/redlib
RUN chmod +x /usr/local/bin/redlib

RUN adduser --home /nonexistent --no-create-home --disabled-password redlib
USER redlib

# Tell Docker to expose port 8080
EXPOSE 8080

# Run a healthcheck every minute to make sure redlib is functional
HEALTHCHECK --interval=1m --timeout=3s CMD wget --spider -q http://localhost:8080/settings || exit 1

CMD ["redlib"]
