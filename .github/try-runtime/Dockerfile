# hadolint global ignore=DL3008
# try-runtime snapshot capture image.
#
# Holds the pinned try-runtime-cli and the capture script. The try-runtime-snapshot CronJob
# in cc-networks-iac runs it inside each network's cluster; CI copies the same binary out
# of it so the CLI is compiled once per pin, not once per pull request.
#
# Build from the repository root:
#   docker build -f .github/try-runtime/Dockerfile .

# Matches rust-toolchain.toml, the toolchain CI has built this CLI revision with.
FROM rust:1.88.0-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0 AS cli-builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends clang libclang-dev protobuf-compiler jq && \
    rm -rf /var/lib/apt/lists/*

COPY .github/try-runtime/config.json /config.json

# The revision comes from config.json, the same pin CI reads, so the two cannot drift.
RUN CLI_REPOSITORY=$(jq -r .cli.repository /config.json) && \
    CLI_REV=$(jq -r .cli.rev /config.json) && \
    cargo install --git "$CLI_REPOSITORY" --rev "$CLI_REV" --locked --root /opt/try-runtime try-runtime-cli && \
    echo "$CLI_REV" > /opt/try-runtime/cli-rev

FROM ubuntu:26.04@sha256:f144425ff09be612d6d9ad965196e9cdc23dae1f42110a8a11a3e9a8198759f7

ARG SOURCE_COMMIT=unknown

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl gnupg jq xxd zstd && \
    rm -rf /var/lib/apt/lists/*

COPY .github/install-azure-cli.sh /tmp/install-azure-cli.sh
RUN /tmp/install-azure-cli.sh && \
    rm -rf /var/lib/apt/lists/* /tmp/install-azure-cli.sh

COPY --from=cli-builder /opt/try-runtime/bin/try-runtime /usr/local/bin/try-runtime
COPY --from=cli-builder /opt/try-runtime/cli-rev /etc/try-runtime/cli-rev
COPY .github/try-runtime/config.json /etc/try-runtime/config.json
COPY .github/try-runtime/capture.sh /usr/local/bin/try-runtime-capture

RUN useradd --create-home --uid 10001 tryruntime && \
    mkdir -p /work && \
    chown tryruntime:tryruntime /work

ENV CONFIG=/etc/try-runtime/config.json \
    WORK_DIR=/work \
    SOURCE_COMMIT=${SOURCE_COMMIT}

USER 10001
WORKDIR /work

ENTRYPOINT ["/usr/local/bin/try-runtime-capture"]
