# Use the base image from subquery-node service
FROM subquerynetwork/subql-node-substrate:v5.11.1

# Switch to root user to install system dependencies
USER root

# Install Python and build tools using apk (Alpine Linux package manager)
RUN apk add --no-cache \
    python3 \
    py3-pip \
    build-base \
    gcc \
    g++ \
    make

# Switch to non-root user *before* setting the working directory
USER node

# Set the working directory inside the container
WORKDIR /app

# Copy the application files (as node user)
COPY --chown=node:node . .

# Build the project (as node user)
RUN yarn && yarn codegen && yarn build

# base image definition is at
# https://github.com/subquery/subql/blob/main/packages/node/Dockerfile
CMD ["-f", "/app", "--db-schema", "app", "--workers", "4", "--batch-size", "30", "--unfinalized-blocks", "true"]
