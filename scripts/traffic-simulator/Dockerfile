# Proof Traffic Simulator Dockerfile
# Uses Deno runtime for native TypeScript support

FROM denoland/deno:2.1.4

# Set working directory
WORKDIR /app

# Copy Deno configuration and lock file first for better caching
COPY deno.json ./
COPY deno.lock* ./

# Cache dependencies by running a dummy import
# This layer will be cached unless deno.json changes
RUN deno cache --reload deno.json || true

# Copy source code
COPY src/ ./src/

# Cache the main application
RUN deno cache src/main.ts

# Create non-root user (deno image already has this)
USER deno

# Expose health check port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
  CMD deno eval --allow-net "const r = await fetch('http://localhost:8080/health'); Deno.exit(r.ok ? 0 : 1);"

# Run the simulator with required permissions
CMD ["run", "--allow-net", "--allow-env", "--allow-read", "src/main.ts"]
