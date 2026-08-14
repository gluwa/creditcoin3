FROM denoland/deno:2.9.5@sha256:b429777c3dcff34a6488f365a1537db1640b2d48379b60f5e6206be034472463

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
