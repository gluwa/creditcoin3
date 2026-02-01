FROM denoland/deno:2.6.7

ENV DENO_DIR=/deno-dir

# Create directories and set ownership before switching to non-root user
RUN mkdir -p /deno-dir /app && chown -R deno:deno /deno-dir /app

# Drop privileges ASAP for security
USER deno

WORKDIR /app

# Copy dependency files first for optimal layer caching
# These change less frequently than source code
COPY --chown=deno:deno scripts/traffic-simulator/deno.json ./
COPY --chown=deno:deno scripts/traffic-simulator/deno.lock* ./

# Cache dependencies (fail on error to catch dependency issues early)
RUN deno cache --reload deno.json

# Copy application source code
COPY --chown=deno:deno scripts/traffic-simulator/src/ ./src/

# Copy ABI file that the symlink points to (need to have the symlink target inside the container)
COPY --chown=deno:deno precompiles/metadata/abi/block_prover.json ./src/abi/block_prover.json

RUN deno cache src/main.ts

# Expose health check port
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
  CMD deno eval --allow-net "const r = await fetch('http://localhost:8080/health'); Deno.exit(r.ok ? 0 : 1);"

# Use ENTRYPOINT to allow CLI arguments to be passed when running the container
# Usage: docker run <image> --source-rpc <url> --private-key <key> ...
ENTRYPOINT ["deno", "run", "--allow-net", "--allow-env", "--allow-read", "src/main.ts"]
CMD []
