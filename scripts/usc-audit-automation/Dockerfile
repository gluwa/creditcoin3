FROM denoland/deno:2.9.7@sha256:fa335acdf6b72106eda2cb6a8cb5f4187e7630e357467489db4b2e7352d5e432

ENV DENO_DIR=/deno-dir

RUN mkdir -p /deno-dir /app && chown -R deno:deno /deno-dir /app

USER deno

WORKDIR /app

COPY --chown=deno:deno scripts/usc-audit-automation/deno.json ./
COPY --chown=deno:deno scripts/usc-audit-automation/deno.lock* ./

RUN deno cache --reload deno.json

COPY --chown=deno:deno scripts/usc-audit-automation/src/ ./src/

ENTRYPOINT ["deno", "run", "--allow-net", "--allow-env", "--allow-read", "src/main.ts"]
CMD []
