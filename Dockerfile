FROM rust:1.76-alpine as builder

RUN apk add --no-cache build-base libc-dev musl-dev

RUN USER=root cargo new --bin bits-and-bytes-web
WORKDIR /bits-and-bytes-web

COPY . .

RUN cargo build --release


FROM debian:buster-slim

COPY --from=builder /bits-and-bytes-web/target/release/bits-and-bytes-web /usr/local/bin/bits-and-bytes-web
COPY --from=builder /bits-and-bytes-web/assets /usr/local/assets

WORKDIR /usr/local

CMD ["bits-and-bytes-web"]
