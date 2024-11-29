# Use the official Rust image from Docker Hub as a base
FROM rust:latest

# Install necessary build tools and dependencies
RUN apt-get update && apt-get install -y \
    git \
    curl \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Update Rust to the latest stable version
RUN rustup update stable

# Set the working directory for the build
WORKDIR /build

# Clone the redlib repository from GitHub
RUN git clone https://github.com/LucifersCircle/redlib.git .

# Build the project using Cargo
RUN cargo build --release

# Expose the required port
EXPOSE 8080

# Set the user to 'redlib' as specified in the original repo
USER redlib

# Command to run when the container starts (start the binary)
CMD ["redlib"]
