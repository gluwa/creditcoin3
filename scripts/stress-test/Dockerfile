FROM denoland/deno:2.7.7

ENV DENO_DIR=/deno-dir

# Create directories and set ownership before switching to non-root user
RUN mkdir -p /deno-dir /app && chown -R deno:deno /deno-dir /app

# Drop privileges ASAP for security
USER deno

WORKDIR /app

# Copy dependency files first for optimal layer caching
# These change less frequently than source code
COPY --chown=deno:deno scripts/stress-test/deno.json ./
COPY --chown=deno:deno scripts/stress-test/deno.lock* ./

# Cache dependencies (fail on error to catch dependency issues early)
RUN deno cache --reload deno.json

# Copy application source code
COPY --chown=deno:deno scripts/stress-test/src/ ./src/

RUN deno cache src/main.ts

# Use ENTRYPOINT to allow CLI arguments to be passed when running the container
# Usage: docker run <image> -m valid -a http://api:3100 -c 1 -s https://rpc.example.com ...
ENTRYPOINT ["deno", "run", "--allow-net", "--allow-env", "--allow-read", "src/main.ts"]
CMD []
