#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
AZURE_RESOURCE_GROUP="${AZURE_RESOURCE_GROUP:-creditcoin-dryrun}"
AKS_CLUSTER_NAME="${AKS_CLUSTER_NAME:-cc3-dryrun-devnet-cluster}"

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

# Function to ensure we're connected to the cluster
ensure_cluster_connection() {
    log "Connecting to AKS cluster..."
    if ! az aks get-credentials \
        --resource-group "$AZURE_RESOURCE_GROUP" \
        --name "$AKS_CLUSTER_NAME" \
        --overwrite-existing 2>/dev/null; then
        error "Failed to connect to cluster. Make sure you're logged in to Azure."
        return 1
    fi
    success "Connected to cluster $AKS_CLUSTER_NAME"
}

# Function to list all dryrun namespaces
list_dryrun_namespaces() {
    log "Listing all dryrun namespaces in cluster $AKS_CLUSTER_NAME..."
    echo ""

    local namespaces
    namespaces=$(kubectl get namespaces -l purpose=dryrun --no-headers 2>/dev/null || true)

    if [ -z "$namespaces" ]; then
        log "No dryrun namespaces found"
        return 0
    fi

    echo "Namespace                          Environment    Deployment ID              Age"
    echo "---------------------------------- -------------- -------------------------- ------"

    while IFS= read -r line; do
        local ns_name=$(echo "$line" | awk '{print $1}')
        local age=$(echo "$line" | awk '{print $3}')
        local env=$(kubectl get namespace "$ns_name" -o jsonpath='{.metadata.labels.environment}' 2>/dev/null || echo "N/A")
        local dep_id=$(kubectl get namespace "$ns_name" -o jsonpath='{.metadata.labels.deployment-id}' 2>/dev/null || echo "N/A")

        printf "%-34s %-14s %-26s %s\n" "$ns_name" "$env" "$dep_id" "$age"
    done <<< "$namespaces"
}

# Function to delete a specific namespace
delete_namespace() {
    local ns_name=$1

    log "Deleting namespace: $ns_name"

    # Check if namespace exists
    if ! kubectl get namespace "$ns_name" &> /dev/null; then
        error "Namespace '$ns_name' not found"
        return 1
    fi

    # Confirm deletion
    read -p "Are you sure you want to delete namespace '$ns_name'? (yes/no): " -r
    echo
    if [[ $REPLY =~ ^[Yy][Ee][Ss]$ ]]; then
        kubectl delete namespace "$ns_name" --wait=true --timeout=300s
        success "Namespace $ns_name deleted successfully"
    else
        warn "Deletion cancelled"
    fi
}

# Function to delete all dryrun namespaces
delete_all_dryrun_namespaces() {
    log "Finding all dryrun namespaces..."

    local ns_list
    ns_list=$(kubectl get namespaces -l purpose=dryrun -o jsonpath='{.items[*].metadata.name}' 2>/dev/null || true)

    if [ -z "$ns_list" ]; then
        log "No dryrun namespaces found"
        return 0
    fi

    log "Found dryrun namespaces:"
    echo "$ns_list" | tr ' ' '\n'
    echo

    # Confirm deletion
    read -p "Delete ALL dryrun namespaces listed above? (yes/no): " -r
    echo
    if [[ $REPLY =~ ^[Yy][Ee][Ss]$ ]]; then
        for ns_name in $ns_list; do
            log "Deleting $ns_name..."
            kubectl delete namespace "$ns_name" --wait=false &
        done
        wait
        success "Deletion initiated for all dryrun namespaces"
    else
        warn "Deletion cancelled"
    fi
}

# Function to check namespace deletion status
check_deletion_status() {
    log "Checking status of namespace deletions..."
    echo ""

    local ns_list
    ns_list=$(kubectl get namespaces -l purpose=dryrun --no-headers 2>/dev/null || true)

    if [ -z "$ns_list" ]; then
        success "No dryrun namespaces found - all cleaned up!"
    else
        log "Namespaces still present:"
        echo ""
        kubectl get namespaces -l purpose=dryrun
    fi
}

# Main function
main() {
    if [ $# -eq 0 ]; then
        echo "Dryrun Network Cleanup Tool"
        echo ""
        echo "Usage: $0 [command] [args]"
        echo ""
        echo "Configuration:"
        echo "  Resource Group: $AZURE_RESOURCE_GROUP"
        echo "  AKS Cluster:    $AKS_CLUSTER_NAME"
        echo ""
        echo "Commands:"
        echo "  list                  List all dryrun namespaces"
        echo "  delete <namespace>    Delete a specific namespace"
        echo "  delete-all            Delete all dryrun namespaces"
        echo "  status                Check deletion status"
        echo ""
        echo "Examples:"
        echo "  $0 list"
        echo "  $0 delete dryrun-dev-12345"
        echo "  $0 delete-all"
        echo "  $0 status"
        echo ""
        echo "Environment Variables:"
        echo "  AZURE_RESOURCE_GROUP  Override resource group (default: creditcoin-dryrun)"
        echo "  AKS_CLUSTER_NAME      Override cluster name (default: cc3-dryrun-devnet-cluster)"
        exit 1
    fi

    local command=$1

    # Ensure cluster connection for all commands
    ensure_cluster_connection || exit 1
    echo ""

    case "$command" in
        list)
            list_dryrun_namespaces
            ;;
        delete)
            if [ $# -ne 2 ]; then
                error "Usage: $0 delete <namespace-name>"
                exit 1
            fi
            delete_namespace "$2"
            ;;
        delete-all)
            delete_all_dryrun_namespaces
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
