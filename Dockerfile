FROM rust:1.71.0-alpine

# Install git and other necessary dependencies using apk
RUN apk update && apk add --no-cache git

# Set the working directory to /build
WORKDIR /build

# Clone the redlib repository
RUN git clone https://github.com/LucifersCircle/redlib.git /build

# Checkout the main branch
RUN cd /build && git checkout main

# Set the final image's default user
RUN adduser --home /nonexistent --no-create-home --disabled-password redlib
USER redlib

# Expose the necessary port
EXPOSE 8080

# Set the default command to run the application
CMD ["redlib"]
