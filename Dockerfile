# hadolint global ignore=DL3008,DL3009,DL3016,SC3046,DL4006,SC2086
FROM ubuntu:24.04 as runtime-base
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    update-ca-certificates && \
    curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get install -y gcc make nodejs --no-install-recommends && \
    npm install -g yarn node-gyp

RUN useradd --home-dir /creditcoin-node --create-home creditcoin
USER creditcoin
SHELL ["/bin/bash", "-c"]
WORKDIR /creditcoin-node


FROM runtime-base AS devel-base
COPY --chown=creditcoin:creditcoin . /creditcoin-node/


FROM devel-base as rust-builder
ARG BUILD_ARGS=""
USER 0
RUN apt-get install -y --no-install-recommends \
    cmake pkg-config libssl-dev git build-essential clang libclang-dev protobuf-compiler
USER creditcoin
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | /bin/sh -s -- -y

COPY --chown=creditcoin:creditcoin . /creditcoin-node/
# shellcheck source=/dev/null
RUN source ~/.cargo/env && \
    cargo build --release ${BUILD_ARGS}


FROM devel-base AS cli-builder
WORKDIR /creditcoin-node/cli
RUN yarn install && yarn build && yarn pack


FROM runtime-base
EXPOSE 30333/tcp
EXPOSE 30333/udp
EXPOSE 9944 9933 9615
ENTRYPOINT [ "/bin/creditcoin3-node" ]

COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/creditcoin3-node /bin/creditcoin3-node
COPY --from=cli-builder  --chown=creditcoin:creditcoin /creditcoin-node/cli/creditcoin-v*.tgz /creditcoin-node/

USER 0
RUN npm install -g /creditcoin-node/creditcoin-v*.tgz

USER creditcoin
RUN mkdir /creditcoin-node/data
