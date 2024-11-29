FROM alpine:3.19

# Set build arguments
ARG TARGET
ARG BUILD_DIR=/build

# Install required dependencies
RUN apk add --no-cache \
    curl \
    git \
    build-base \
    cmake

# Clone the repository and checkout the main branch
RUN git clone https://github.com/LucifersCircle/redlib.git ${BUILD_DIR} && \
    cd ${BUILD_DIR} && \
    git checkout main

# Build the project
WORKDIR ${BUILD_DIR}

# Run cmake to configure and make to build the project
RUN cmake -DCMAKE_BUILD_TYPE=Release -DTARGET=${TARGET} . && \
    make

# Move the compiled binary to /usr/local/bin
RUN mv ${BUILD_DIR}/target/${TARGET}/redlib /usr/local/bin/redlib

# Clean up build dependencies
RUN apk del build-base cmake && rm -rf ${BUILD_DIR}

# Add a non-root user for security
RUN adduser --home /nonexistent --no-create-home --disabled-password redlib
USER redlib

# Expose port 8080
EXPOSE 8080

# Healthcheck to ensure redlib is functional
HEALTHCHECK --interval=1m --timeout=3s CMD wget --spider -q http://localhost:8080/settings || exit 1

CMD ["redlib"]
