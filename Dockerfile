FROM rust:1.71.0-alpine

# Set the target architecture as an ARG (default to x86_64)
ARG TARGET=x86_64-unknown-linux-musl

# Install dependencies (curl, make, cmake, etc.)
RUN apk add --no-cache curl cmake make

# Create the working directory for the build process
WORKDIR /build

# Clone the redlib repository
RUN git clone https://github.com/LucifersCircle/redlib.git /build

# Checkout the main branch
RUN cd /build && git checkout main

# Build the redlib binary using cargo
RUN cd /build && cargo build --release --target ${TARGET}

# Copy the binary to the appropriate location
RUN cp /build/target/${TARGET}/release/redlib /usr/local/bin/redlib

# Set up a non-privileged user to run the binary
RUN adduser --home /nonexistent --no-create-home --disabled-password redlib
USER redlib

# Expose port 8080
EXPOSE 8080

# Run a healthcheck every minute to make sure redlib is functional
HEALTHCHECK --interval=1m --timeout=3s CMD wget --spider -q http://localhost:8080/settings || exit 1

# Command to run the binary
CMD ["redlib"]
