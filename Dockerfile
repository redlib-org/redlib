# Build stage
FROM rust:latest AS builder

# Install musl target for static linking
RUN rustup target add x86_64-unknown-linux-musl

# Set the working directory
WORKDIR /build

# Clone the repository
RUN git clone https://github.com/LucifersCircle/redlib.git .

# Build the project with musl target
RUN cargo build --release --target=x86_64-unknown-linux-musl

# Final stage with minimal base image
FROM alpine:latest

# Copy the statically linked binary from the builder stage
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/redlib /usr/local/bin/redlib

# Set the working directory
WORKDIR /app

# Expose the application port
EXPOSE 8080

# Run the binary
CMD ["redlib"]
