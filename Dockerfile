FROM rust:1.76-alpine as builder

RUN apk add --no-cache build-base libc-dev musl-dev openssl-dev

ENV OPENSSL_LIB_DIR=/usr/lib
ENV OPENSSL_INCLUDE_DIR=/usr/include

RUN USER=root cargo new --bin bits-and-bytes-web
WORKDIR /bits-and-bytes-web

COPY . .

RUN cargo build --release


FROM debian:buster-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /bits-and-bytes-web/target/release/bits-and-bytes-web /usr/local/bin/bits-and-bytes-web
COPY --from=builder /bits-and-bytes-web/assets /usr/local/assets

WORKDIR /usr/local

CMD ["bits-and-bytes-web"]
