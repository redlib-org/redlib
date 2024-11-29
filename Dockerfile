# Build stage
FROM rust:latest AS builder

# Install dependencies
RUN apt-get update && apt-get install -y git

# Set the working directory
WORKDIR /build

# Clone the repository
RUN git clone https://github.com/LucifersCircle/redlib.git .

# Build the project using Cargo
RUN cargo build --release

# Final stage
FROM alpine:latest

# Install required runtime libraries
RUN apk add --no-cache libc6-compat

# Copy the compiled binary from the builder stage
COPY --from=builder /build/target/release/redlib /usr/local/bin/redlib

# Set the working directory
WORKDIR /app

# Expose the application port (update if needed)
EXPOSE 8080

# Run the binary
CMD ["redlib"]
