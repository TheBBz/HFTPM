FROM rust:1.88-slim as builder

WORKDIR /app

RUN apt update && apt install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    git \
    curl

COPY . .

RUN cargo install --locked --path . --profile release

FROM debian:12-slim

RUN apt update && apt install -y \
    libssl3 \
    ca-certificates \
    tmux \
    htop \
    curl \
    wget

WORKDIR /app

COPY --from=builder /app/target/release/hfptm /app/
COPY --from=builder /app/config /app/config/

RUN useradd -m -s /bin/bash hfptm

USER hfptm

ENV RUST_LOG=info
ENV PATH="/app:$PATH"

EXPOSE 3000

CMD ["/app/hfptm"]
