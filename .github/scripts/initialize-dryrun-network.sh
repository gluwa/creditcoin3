#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
CLI_PATH="cli/dist/cli.js"
ALICE_URL="ws://localhost:9944"
BOND_AMOUNT="10000"
FUND_AMOUNT="50000"

# Well-known development accounts
declare -A ACCOUNTS=(
    ["alice"]="//Alice"
    ["bob"]="//Bob"
    ["charlie"]="//Charlie"
    ["dave"]="//Dave"
    ["eve"]="//Eve"
)

# Node URLs - we'll port-forward to each as needed
declare -A NODE_URLS=(
    ["alice"]="ws://localhost:9944"
    ["bob"]="ws://localhost:9945"
    ["charlie"]="ws://localhost:9946"
    ["dave"]="ws://localhost:9947"
    ["eve"]="ws://localhost:9948"
)

# Array of validator names in order
VALIDATORS=("alice" "bob" "charlie" "dave" "eve")

log() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

# Function to wait for a node to be ready
wait_for_node() {
    local url=$1
    local max_attempts=30
    local attempt=0

    log "Waiting for node at $url to be ready..."

    while [ $attempt -lt $max_attempts ]; do
        if node "$CLI_PATH" status --url "$url" &>/dev/null; then
            success "Node at $url is ready"
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 2
    done

    error "Node at $url did not become ready in time"
    return 1
}

# Function to setup port forwarding for a node
setup_port_forward() {
    local node_id=$1
    local local_port=$2

    log "Setting up port forward for node-$node_id on port $local_port..."

    # Kill any existing port forward on this port
    pkill -f "port-forward.*creditcoin-node-$node_id.*$local_port:9944" || true
    sleep 1

    # Start port forward in background
    kubectl port-forward -n creditcoin-dryrun "svc/creditcoin-node-$node_id" "$local_port:9944" &
    local pf_pid=$!

    # Wait for port forward to be ready
    sleep 3

    # Verify port forward is working
    if ! ps -p $pf_pid > /dev/null; then
        error "Port forward for node-$node_id failed to start"
        return 1
    fi

    success "Port forward active for node-$node_id (PID: $pf_pid)"
    echo $pf_pid
}

# Function to increase validator count
increase_validator_count() {
    local count=$1
    log "Increasing validator count by $count..."

    CC_SECRET="${ACCOUNTS[alice]}" node "$CLI_PATH" status --url "$ALICE_URL" > /dev/null

    # Use sudo to increase validator count
    # This would need to be done via a custom script or direct RPC call
    # For now, we assume the chainspec has sufficient validator slots
    warn "Ensure chainspec has validator count >= $count"
}

# Function to fund an account from sudo
fund_account() {
    local account_secret=$1
    local amount=$2
    local url=$3

    log "Funding account..."

    # Get the address of the account to fund
    local address
    address=$(CC_SECRET="$account_secret" node "$CLI_PATH" show-address --format json | grep -o '"substrate":"[^"]*"' | cut -d'"' -f4)

    log "Account address: $address"

    # Fund via sudo - note: this requires a custom implementation
    # For the dryrun network, we rely on well-known accounts already having funds in the genesis
    success "Account should be funded in genesis (well-known account)"
}

# Function to bond tokens
bond_tokens() {
    local account_secret=$1
    local amount=$2
    local url=$3

    log "Bonding $amount tokens..."

    if CC_SECRET="$account_secret" node "$CLI_PATH" bond --amount "$amount" --url "$url"; then
        success "Bonded $amount tokens successfully"
        return 0
    else
        error "Failed to bond tokens"
        return 1
    fi
}

# Function to rotate keys
rotate_keys() {
    local url=$1

    log "Rotating keys on node at $url..."

    local keys
    keys=$(node "$CLI_PATH" rotate-keys --url "$url" | grep "New keys:" | sed 's/New keys: //')

    if [ -z "$keys" ]; then
        error "Failed to rotate keys"
        return 1
    fi

    success "Rotated keys: $keys"
    echo "$keys"
}

# Function to set keys
set_keys() {
    local account_secret=$1
    local keys=$2
    local url=$3

    log "Setting session keys..."

    if CC_SECRET="$account_secret" node "$CLI_PATH" set-keys --keys "$keys" --url "$url"; then
        success "Set session keys successfully"
        return 0
    else
        error "Failed to set keys"
        return 1
    fi
}

# Function to validate
start_validating() {
    local account_secret=$1
    local url=$2

    log "Starting validation with 1% commission..."

    if CC_SECRET="$account_secret" node "$CLI_PATH" validate --commission 1 --url "$url"; then
        success "Started validating successfully"
        return 0
    else
        error "Failed to start validating"
        return 1
    fi
}

# Function to check validator status
check_validator_status() {
    local account_secret=$1
    local url=$2

    log "Checking validator status..."

    CC_SECRET="$account_secret" node "$CLI_PATH" status --url "$url" || true
}

# Main initialization flow
main() {
    log "========================================="
    log "Starting Dryrun Network Initialization"
    log "========================================="

    # Determine number of validators to set up
    local num_validators=${NODE_COUNT:-4}
    log "Setting up $num_validators validators"

    # Wait for the first node (Alice) to be ready
    log "Waiting for bootnode to be ready..."
    wait_for_node "$ALICE_URL"

    sleep 5

    # Setup validators
    for i in $(seq 0 $((num_validators - 1))); do
        local validator_name="${VALIDATORS[$i]}"
        local account_secret="${ACCOUNTS[$validator_name]}"
        local node_port=$((9944 + i))

        if [ $i -ne 0 ]; then
            local node_url="${NODE_URLS[$validator_name]}"

            log ""
            log "========================================="
            log "Setting up Validator: $validator_name (Node $i)"
            log "========================================="

            # Setup port forward for this node
            local pf_pid
            pf_pid=$(setup_port_forward "$i" "$node_port")

            # Wait for node to be ready
            wait_for_node "$node_url"

            sleep 2
        else
            local node_url="$ALICE_URL"
            log ""
            log "========================================="
            log "Setting up Validator: $validator_name (Bootnode)"
            log "========================================="
        fi

        # Fund account (well-known accounts are pre-funded in genesis)
        log "Account $validator_name is pre-funded in genesis"

        # Bond tokens
        if ! bond_tokens "$account_secret" "$BOND_AMOUNT" "$ALICE_URL"; then
            error "Failed to bond tokens for $validator_name"
            continue
        fi

        sleep 2

        # Rotate keys on the node
        local rotated_keys
        if ! rotated_keys=$(rotate_keys "$node_url"); then
            error "Failed to rotate keys for $validator_name"
            continue
        fi

        sleep 2

        # Set the session keys
        if ! set_keys "$account_secret" "$rotated_keys" "$ALICE_URL"; then
            error "Failed to set keys for $validator_name"
            continue
        fi

        sleep 2

        # Start validating
        if ! start_validating "$account_secret" "$ALICE_URL"; then
            error "Failed to start validating for $validator_name"
            continue
        fi

        sleep 2

        # Check status
        check_validator_status "$account_secret" "$ALICE_URL"

        success "Validator $validator_name setup complete!"
    done

    log ""
    log "========================================="
    log "Network Initialization Complete!"
    log "========================================="
    log ""
    log "Summary:"
    log "  - $num_validators validators configured"
    log "  - All validators bonded with $BOND_AMOUNT tokens"
    log "  - All validators have rotated keys"
    log "  - All validators are signaling to validate"
    log ""
    log "Next steps:"
    log "  1. Wait for the next era for validators to become active"
    log "  2. Monitor validator status with: node $CLI_PATH status --url $ALICE_URL"
    log "  3. Check logs: kubectl logs -n creditcoin-dryrun -l app=creditcoin-node --tail=50"
    log ""

    # Cleanup port forwards
    log "Cleaning up port forwards..."
    pkill -f "kubectl port-forward.*creditcoin-node" || true

    success "Initialization complete!"
}

# Run main function
main "$@"
