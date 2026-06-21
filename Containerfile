FROM docker.io/rust:1.96 AS base

WORKDIR /app

ARG TARGETARCH
ARG IMAPTEST_RELEASE=latest

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        openssl \
    && rm -rf /var/lib/apt/lists/*

RUN set -eu; \
    case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
        amd64) imaptest_arch="x86_64" ;; \
        arm64) imaptest_arch="aarch64" ;; \
        *) echo "Unsupported imaptest architecture: ${TARGETARCH:-$(dpkg --print-architecture)}" >&2; exit 1 ;; \
    esac; \
    imaptest_asset="imaptest-${imaptest_arch}-debian-13"; \
    imaptest_base_url="https://github.com/dovecot/imaptest/releases/download/${IMAPTEST_RELEASE}"; \
    curl -fsSLo "/tmp/${imaptest_asset}" "${imaptest_base_url}/${imaptest_asset}"; \
    curl -fsSLo /tmp/imaptest-SHA256SUMS.txt "${imaptest_base_url}/SHA256SUMS.txt"; \
    cd /tmp; \
    grep "  ${imaptest_asset}$" imaptest-SHA256SUMS.txt > /tmp/imaptest.sha256 \
        || { echo "Missing checksum entry for ${imaptest_asset}" >&2; exit 1; }; \
    sha256sum -c /tmp/imaptest.sha256; \
    install -m 0755 "/tmp/${imaptest_asset}" /usr/local/bin/imaptest; \
    rm -f "/tmp/${imaptest_asset}" /tmp/imaptest-SHA256SUMS.txt /tmp/imaptest.sha256

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests
COPY bin ./bin

FROM base AS test
RUN cargo test --locked

FROM base AS build
RUN cargo build --locked

FROM build AS runtime
ENV IMAP_BIND_ADDR=0.0.0.0:1143
EXPOSE 1143
CMD ["./target/debug/mail"]
