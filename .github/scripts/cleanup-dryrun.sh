#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

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

# Function to list all dryrun resource groups
list_dryrun_resources() {
    log "Listing all dryrun resource groups..."

    az group list \
        --query "[?tags.purpose=='dryrun'].{Name:name, Environment:tags.environment, Created:tags.created_by, RunID:tags.run_id}" \
        --output table
}

# Function to delete a specific resource group
delete_resource_group() {
    local rg_name=$1

    log "Deleting resource group: $rg_name"

    # Confirm deletion
    read -p "Are you sure you want to delete resource group '$rg_name'? (yes/no): " -r
    echo
    if [[ $REPLY =~ ^[Yy][Ee][Ss]$ ]]; then
        az group delete --name "$rg_name" --yes --no-wait
        success "Deletion initiated for $rg_name (running in background)"
    else
        warn "Deletion cancelled"
    fi
}

# Function to delete all dryrun resource groups
delete_all_dryrun_resources() {
    log "Finding all dryrun resource groups..."

    local rg_list
    rg_list=$(az group list --query "[?tags.purpose=='dryrun'].name" --output tsv)

    if [ -z "$rg_list" ]; then
        log "No dryrun resource groups found"
        return 0
    fi

    log "Found dryrun resource groups:"
    echo "$rg_list"
    echo

    # Confirm deletion
    read -p "Delete ALL dryrun resource groups listed above? (yes/no): " -r
    echo
    if [[ $REPLY =~ ^[Yy][Ee][Ss]$ ]]; then
        while IFS= read -r rg_name; do
            log "Deleting $rg_name..."
            az group delete --name "$rg_name" --yes --no-wait
        done <<< "$rg_list"
        success "Deletion initiated for all dryrun resource groups"
    else
        warn "Deletion cancelled"
    fi
}

# Function to check deletion status
check_deletion_status() {
    log "Checking status of resource group deletions..."

    local rg_list
    rg_list=$(az group list --query "[?tags.purpose=='dryrun'].{Name:name, ProvisioningState:properties.provisioningState}" --output table)

    if [ -z "$rg_list" ]; then
        success "No dryrun resource groups found - all cleaned up!"
    else
        log "Current status:"
        echo "$rg_list"
    fi
}

# Main function
main() {
    if [ $# -eq 0 ]; then
        echo "Usage: $0 [command] [args]"
        echo ""
        echo "Commands:"
        echo "  list                     List all dryrun resource groups"
        echo "  delete <resource-group>  Delete a specific resource group"
        echo "  delete-all               Delete all dryrun resource groups"
        echo "  status                   Check deletion status"
        echo ""
        echo "Examples:"
        echo "  $0 list"
        echo "  $0 delete creditcoin-dryrun-12345"
        echo "  $0 delete-all"
        echo "  $0 status"
        exit 1
    fi

    local command=$1

    case "$command" in
        list)
            list_dryrun_resources
            ;;
        delete)
            if [ $# -ne 2 ]; then
                error "Usage: $0 delete <resource-group-name>"
                exit 1
            fi
            delete_resource_group "$2"
            ;;
        delete-all)
            delete_all_dryrun_resources
            ;;
        status)
            check_deletion_status
            ;;
        *)
            error "Unknown command: $command"
            exit 1
            ;;
    esac
}

main "$@"
