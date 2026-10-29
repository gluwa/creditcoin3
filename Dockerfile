# hadolint global ignore=DL3008,DL3009,DL3013,DL3016,SC3046,DL4006,SC1091,SC2086
FROM ubuntu:26.04@sha256:f144425ff09be612d6d9ad965196e9cdc23dae1f42110a8a11a3e9a8198759f7 AS runtime-base
ENV DEBIAN_FRONTEND=noninteractive
ARG TARGETARCH
ARG NODE_VERSION=24.18.0
# NodeSource's apt key fetch (deb.nodesource.com) intermittently 403s from our
# self-hosted Linode CI runners' shared IP pool. Install the official upstream
# release directly instead, checksum-verified against nodejs.org's own SHASUMS256.txt.
RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install -y --no-install-recommends ca-certificates curl xz-utils libdw1t64 libpq5 && \
    update-ca-certificates && \
    case "${TARGETARCH}" in \
      amd64) NODE_ARCH=x64;    NODE_SHA256=55aa7153f9d88f28d765fcdad5ae6945b5c0f98a36881703817e4c450fa76742 ;; \
      arm64) NODE_ARCH=arm64;  NODE_SHA256=58c9520501f6ae2b52d5b210444e24b9d0c029a58c5011b797bc1fe7105886f6 ;; \
      *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac && \
    curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${NODE_ARCH}.tar.xz" -o /tmp/node.tar.xz && \
    echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum -c - && \
    tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 --no-same-owner && \
    rm /tmp/node.tar.xz && \
    node --version && \
    npm --version && \
    npm install -g yarn node-gyp
# WARNING: devel dependencies should go into the devel-base image below

RUN useradd --home-dir /creditcoin-node --create-home creditcoin
USER creditcoin
SHELL ["/bin/bash", "-c"]
WORKDIR /creditcoin-node


FROM runtime-base AS devel-base
USER 0
# NOTE: only devel releated dependencies here
RUN apt-get install -y --no-install-recommends \
    software-properties-common \
    gcc libpq-dev make jq
COPY --chown=creditcoin:creditcoin . /creditcoin-node/

USER creditcoin


FROM devel-base AS rust-builder
ARG BUILD_ARGS="--features metadata-hash"
USER 0
RUN apt-get install -y --no-install-recommends \
    cmake pkg-config libssl-dev git build-essential clang libclang-dev protobuf-compiler
USER creditcoin
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | /bin/sh -s -- -y

# Ubuntu 26.04 ships GCC 15, whose libstdc++ no longer transitively includes
# <cstdint>. The bundled RocksDB 8.1.1 headers (librocksdb-sys 0.11.0+8.1.1)
# use uint64_t without including it, so force-include cstdint when compiling C/C++.
ENV CXXFLAGS="-include cstdint"

COPY --chown=creditcoin:creditcoin . /creditcoin-node/
# shellcheck source=/dev/null
RUN source ~/.cargo/env && \
    rustup toolchain install && \
    cargo build --release ${BUILD_ARGS}


FROM devel-base AS cli-builder
WORKDIR /creditcoin-node/precompiles/metadata

WORKDIR /creditcoin-node/docs/smart-contract-development/with-hardhat
RUN npm install && npx hardhat compile

WORKDIR /creditcoin-node/cli
RUN yarn install && yarn build && yarn pack


FROM runtime-base AS packaged-base
EXPOSE 30333/tcp
EXPOSE 30333/udp
EXPOSE 9944 9933 9615
ENTRYPOINT [ "/bin/creditcoin3-node" ]

COPY --from=cli-builder  --chown=creditcoin:creditcoin /creditcoin-node/cli/creditcoin-v*.tgz /creditcoin-node/
COPY --chown=creditcoin:creditcoin chainspecs /

USER 0
RUN npm install -g /creditcoin-node/creditcoin-v*.tgz

USER creditcoin
RUN mkdir /creditcoin-node/data


# Releases supply this named context from the native build in the same workflow run.
# Artifact downloads do not preserve executable permissions.
FROM packaged-base AS prebuilt-image
# hadolint ignore=DL3022
COPY --chmod=0755 --chown=creditcoin:creditcoin --from=release-artifacts \
    /creditcoin3-node /attestor /attestor_zombienet /proof-gen-api-server /archiver /query-cli /bin/


# Keep source compilation as the default for standalone/nightly Docker builds.
FROM packaged-base AS final
COPY --chown=creditcoin:creditcoin --from=rust-builder \
    /creditcoin-node/target/release/creditcoin3-node \
    /creditcoin-node/target/release/attestor \
    /creditcoin-node/target/release/attestor_zombienet \
    /creditcoin-node/target/release/proof-gen-api-server \
    /creditcoin-node/target/release/archiver \
    /creditcoin-node/target/release/query-cli /bin/
