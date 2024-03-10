# Use the official Rust image as the base image
FROM rust:1.76-alpine as builder

RUN apk add --no-cache build-base libc-dev musl-dev

# Create a new empty shell project
RUN USER=root cargo new --bin bits-and-bytes-web
WORKDIR /bits-and-bytes-web

# Copy the Cargo.toml and Cargo.lock files into the new project
# This is a separate step so the dependencies will be cached unless
# these files change.
COPY . .

# Build only the dependencies to cache them
RUN cargo build --release


# The final image will be based on the debian buster-slim image
FROM debian:buster-slim

# Copy the binary from the builder stage to the final stage
COPY --from=builder /bits-and-bytes-web/target/release/bits-and-bytes-web /usr/local/bin/bits-and-bytes-web

# Set the default command to run your binary
CMD ["bits-and-bytes-web"]
