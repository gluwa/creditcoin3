# hadolint global ignore=DL3008,DL3009,SC3046,DL4006

FROM ubuntu:22.04 as runtime-base
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    update-ca-certificates

RUN useradd --home-dir /creditcoin-node --create-home creditcoin
USER creditcoin
SHELL ["/bin/bash", "-c"]
WORKDIR /creditcoin-node


FROM runtime-base AS devel-base
COPY --chown=creditcoin:creditcoin . /creditcoin-node/


FROM devel-base as rust-builder
USER 0
RUN apt-get install -y --no-install-recommends \
    cmake pkg-config libssl-dev git build-essential clang libclang-dev protobuf-compiler
USER creditcoin
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | /bin/sh -s -- -y

COPY --chown=creditcoin:creditcoin . /creditcoin-node/
# shellcheck source=/dev/null
RUN source ~/.cargo/env && \
    cargo build --release


FROM runtime-base
EXPOSE 30333/tcp
EXPOSE 30333/udp
EXPOSE 9944 9933 9615
ENTRYPOINT [ "/bin/creditcoin-node" ]

COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/frontier-template-node /bin/creditcoin-node

USER creditcoin
RUN mkdir /creditcoin-node/data
