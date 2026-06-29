# Use debian:bullseye-slim as base image since it has glibc 2.31
FROM --platform=linux/amd64 debian:bullseye-slim

# Install required packages
RUN apt-get update && apt-get install -y \
    curl \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user to run the application
RUN groupadd -r distri && useradd -r -g distri distri

# Create directory for the application
WORKDIR /app

# Copy the binary from the build output
COPY target/x86_64-unknown-linux-gnu/release/distri /app/

# Set ownership of the application files
RUN chown -R distri:distri /app

# Switch to non-root user
USER distri

# Expose the port (assuming default HTTP port 8081 - adjust if needed)
EXPOSE 8081

# Run the binary
ENTRYPOINT ["/app/distri"] 