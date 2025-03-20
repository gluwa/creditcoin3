FROM postgres:16-alpine

# Create a non-root user and group
RUN addgroup -S postgresgroup && adduser -S postgresuser -G postgresgroup

# Ensure postgresuser owns the necessary directories
RUN chown -R postgresuser:postgresgroup /var/lib/postgresql /var/run/postgresql

# Temporarily run as root to allow initialization
USER root

# Copy init scripts (must be done as root to avoid permission issues)
COPY postgres/load-extensions.sh /docker-entrypoint-initdb.d/

# Switch to non-root user for security compliance
USER postgresuser

# Environment variables for PostgreSQL
ENV POSTGRES_DB=postgres
ENV POSTGRES_USER=postgres
ENV POSTGRES_PASSWORD=postgres
