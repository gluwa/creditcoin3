# hadolint global ignore=DL3008,DL3009,DL3013,DL3016,SC3046,DL4006,SC1091,SC2086
FROM ubuntu:24.04 AS runtime-base
ENV DEBIAN_FRONTEND=noninteractive
COPY ./deadsnakes-ubuntu-ppa-noble.sources /etc/apt/sources.list.d/
RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    update-ca-certificates && \
    curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get install -y libdw1t64 libpq5 nodejs python3.10 --no-install-recommends && \
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
    gcc libgmp-dev libpq-dev make python3.10-dev python3-virtualenv
USER creditcoin
COPY --chown=creditcoin:creditcoin . /creditcoin-node/
ENV PATH=/creditcoin-node/venv/bin:${PATH} \
    VIRTUAL_ENV=/creditcoin-node/venv
RUN virtualenv --python /usr/bin/python3.10 /creditcoin-node/venv
RUN source /creditcoin-node/venv/bin/activate && \
    pip install --no-cache-dir --upgrade setuptools && \
    pip install --no-cache-dir --requirement /creditcoin-node/prover/requirements.txt

RUN cairo-compile /creditcoin-node/cairo/scripts/verify_merkle_proof.cairo \
         --output /creditcoin-node/cairo/scripts/verify_merkle_proof.cairo_compiled.json --proof_mode 2>&1


FROM devel-base AS rust-builder
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

COPY --from=cli-builder  --chown=creditcoin:creditcoin /creditcoin-node/cli/creditcoin-v*.tgz /creditcoin-node/
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/creditcoin3-node /bin/creditcoin3-node
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/chainspecs /

COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/attestor /bin/attestor
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/attestor_zombienet /bin/attestor_zombienet
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/query-cli /bin/query-cli
COPY --from=rust-builder --chown=creditcoin:creditcoin /creditcoin-node/target/release/prover /bin/prover

ENV PATH=/creditcoin-node/venv/bin:/cairo/scripts:/cairo/stone-prover:/cairo/stone-verifier:${PATH} \
    VIRTUAL_ENV=/creditcoin-node/venv
COPY --from=devel-base --chown=creditcoin:creditcoin /creditcoin-node/venv/ /creditcoin-node/venv
COPY --from=devel-base --chown=creditcoin:creditcoin /creditcoin-node/cairo/ /cairo

USER 0
RUN npm install -g /creditcoin-node/creditcoin-v*.tgz

USER creditcoin
RUN mkdir /creditcoin-node/data
