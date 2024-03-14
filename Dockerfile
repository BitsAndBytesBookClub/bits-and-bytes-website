FROM rust:1.76 as builder

RUN apt-get update && apt-get install -y \
    build-essential \
    libc6-dev \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN USER=root cargo new --bin bits-and-bytes-web
WORKDIR /bits-and-bytes-web

COPY . .

RUN cargo build --release


FROM ubuntu:24.04

RUN apt-get update && apt-get install -y ca-certificates openssl && rm -rf /var/lib/apt/lists/*
RUN apt-get update && apt-get install -y \
    build-essential \
    libc6-dev \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /bits-and-bytes-web/target/release/bits-and-bytes-web /usr/local/bin/bits-and-bytes-web
COPY --from=builder /bits-and-bytes-web/assets /usr/local/assets

WORKDIR /usr/local

ENV RUST_LOG debug

CMD ["bits-and-bytes-web"]
