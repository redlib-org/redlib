FROM rust:1.71.0-alpine3.19

# Set the target architecture as an ARG
ARG TARGET

# Install necessary dependencies for building (including git)
RUN apk add --no-cache curl git build-base

# Clone the repository into /redlib
RUN git clone https://github.com/LucifersCircle/redlib.git /redlib

# Set the working directory to the root of the cloned repository
WORKDIR /redlib

# Checkout the main branch (if needed)
RUN git checkout main

# Build the project using Cargo (this will handle the Rust-specific build)
RUN cargo build --release --target ${TARGET}

# Create a non-root user to run the application
RUN adduser --home /nonexistent --no-create-home --disabled-password redlib
USER redlib

# Expose the application port (if applicable)
EXPOSE 8080

# Run a healthcheck every minute to make sure redlib is functional
HEALTHCHECK --interval=1m --timeout=3s CMD wget --spider -q http://localhost:8080/settings || exit 1

# Default command to run the application
CMD ["./target/${TARGET}/release/redlib"]
