FROM denoland/deno:2.7.8

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
