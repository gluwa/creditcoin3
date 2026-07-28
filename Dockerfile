# hadolint global ignore=DL3008,DL3009,DL3013,DL3016,SC3046,DL4006,SC1091,SC2086
FROM ubuntu:26.04@sha256:b7f48194d4d8b763a478a621cdc81c27be222ba2206ca3ca6bc42b49685f3d9e AS runtime-base
ENV DEBIAN_FRONTEND=noninteractive
ARG TARGETARCH
ARG NODE_VERSION=22.23.1
# NodeSource's apt key fetch (deb.nodesource.com) intermittently 403s from our
# self-hosted Linode CI runners' shared IP pool. Install the official upstream
# release directly instead, checksum-verified against nodejs.org's own SHASUMS256.txt.
RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install -y --no-install-recommends ca-certificates curl xz-utils libdw1t64 libpq5 && \
    update-ca-certificates && \
    case "${TARGETARCH}" in \
      amd64) NODE_ARCH=x64;    NODE_SHA256=9749e988f437343b7fa832c69ded82a312e41a03116d766797ac14f6f9eee578 ;; \
      arm64) NODE_ARCH=arm64;  NODE_SHA256=0294e8b915ab75f92c7513d2fcb830ae06e10684e6c603e99a87dbf8835389c1 ;; \
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


FROM runtime-base
EXPOSE 30333/tcp
EXPOSE 30333/udp
EXPOSE 9944 9933 9615
ENTRYPOINT [ "/bin/creditcoin3-node" ]

COPY --from=cli-builder  --chown=creditcoin:creditcoin /creditcoin-node/cli/creditcoin-v*.tgz /creditcoin-node/
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/creditcoin3-node /bin/creditcoin3-node
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/chainspecs /

COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/attestor /bin/attestor
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/attestor_zombienet /bin/attestor_zombienet
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/proof-gen-api-server /bin/proof-gen-api-server
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/archiver /bin/archiver
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/query-cli /bin/query-cli

USER 0
RUN npm install -g /creditcoin-node/creditcoin-v*.tgz

USER creditcoin
RUN mkdir /creditcoin-node/data
