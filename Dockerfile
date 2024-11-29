FROM alpine:3.19

ARG TARGET=x86_64-unknown-linux-musl

RUN apk add --no-cache curl tar

# Download the latest artifact from the main branch and extract it
RUN curl -L "https://github.com/LucifersCircle/redlib/archive/refs/heads/main.tar.gz" | \
    tar --strip-components=1 -xz -C /usr/local/bin/ redlib-main/target/${TARGET}/redlib

RUN adduser --home /nonexistent --no-create-home --disabled-password redlib
USER redlib

# Tell Docker to expose port 8080
EXPOSE 8080

# Run a healthcheck every minute to make sure redlib is functional
HEALTHCHECK --interval=1m --timeout=3s CMD wget --spider -q http://localhost:8080/settings || exit 1

CMD ["redlib"]
