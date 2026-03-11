# Build stage
FROM rust:1.85 as builder

WORKDIR /app

# Copy manifest files first
COPY Cargo.toml ./

# Create a dummy main.rs to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies
RUN cargo build --release
RUN rm -rf src

# Copy actual source code
COPY src ./src

# Touch main.rs to ensure it's rebuilt
RUN touch src/main.rs

# Build the application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install necessary runtime dependencies including FFmpeg
RUN apt-get update && \
    apt-get install -y ca-certificates ffmpeg && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/backend /app/backend

# Create directory for videos and database
RUN mkdir -p /app/videos

# Expose the port your Rust app runs on
EXPOSE 3000

# Run the binary
CMD ["./backend"]