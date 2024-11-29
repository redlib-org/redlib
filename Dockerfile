FROM alpine:3.19

ARG TARGET=x86_64-unknown-linux-musl
ARG BUILD_DIR=/build

RUN apk add --no-cache curl build-base cmake

# Download the latest source code from the main branch
RUN mkdir -p ${BUILD_DIR} && \
    curl -L "https://github.com/LucifersCircle/redlib/archive/refs/heads/main.tar.gz" | \
    tar --strip-components=1 -xz -C ${BUILD_DIR}

# Build the binary
WORKDIR ${BUILD_DIR}
RUN cmake -DCMAKE_BUILD_TYPE=Release -DTARGET=${TARGET} . && \
    make

# Move the compiled binary to /usr/local/bin
RUN mv ${BUILD_DIR}/target/${TARGET}/redlib /usr/local/bin/redlib

# Clean up build dependencies
RUN apk del build-base cmake && rm -rf ${BUILD_DIR}

RUN adduser --home /nonexistent --no-create-home --disabled-password redlib
USER redlib

EXPOSE 8080

HEALTHCHECK --interval=1m --timeout=3s CMD wget --spider -q http://localhost:8080/settings || exit 1

CMD ["redlib"]
