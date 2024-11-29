# Use an Alpine-based image as the base
FROM rust:1.74.0-alpine AS build

# Install necessary dependencies (git, curl, etc.)
RUN apk add --no-cache git curl build-base

# Upgrade Cargo to the latest version
RUN rustup update stable

# Set the working directory for the build process
WORKDIR /build

# Clone the repository
RUN git clone https://github.com/LucifersCircle/redlib.git .

# Build the project using Cargo
RUN cargo build --release

# Final image with only the necessary runtime dependencies
FROM alpine:3.19

# Install dependencies needed to run the binary (curl, for healthcheck)
RUN apk add --no-cache curl

# Copy the compiled binary from the build stage
COPY --from=build /build/target/release/redlib /usr/local/bin/

# Add a user to run the application
RUN adduser --home /nonexistent --no-create-home --disabled-password redlib
USER redlib

# Expose the necessary port
EXPOSE 8080

# Run a healthcheck to ensure the app is working
HEALTHCHECK --interval=1m --timeout=3s CMD wget --spider -q http://localhost:8080/settings || exit 1

# Set the default command to run the redlib binary
CMD ["redlib"]
