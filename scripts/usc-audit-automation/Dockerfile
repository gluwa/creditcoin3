FROM denoland/deno:2.9.8@sha256:a0d88a7fcbd44f062cba2748e19df67d30290ad12aa203c7c1e0b12462c483f0

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
