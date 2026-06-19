FROM docker.io/rust:1.96 AS base

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libsqlite3-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src

FROM base AS test
RUN cargo test --locked

FROM base AS build
RUN cargo build --locked

FROM build AS runtime
ENV IMAP_BIND_ADDR=0.0.0.0:1143
EXPOSE 1143
CMD ["./target/debug/mail"]
