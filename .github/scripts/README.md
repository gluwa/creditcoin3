# Dryrun Network Scripts

This directory contains scripts for managing Creditcoin3 dryrun networks on Azure Kubernetes Service.

## Scripts

### initialize-dryrun-network.sh

Initializes a dryrun blockchain network by:
- Funding validator accounts
- Bonding tokens for validators
- Rotating session keys
- Setting session keys for each validator
- Starting validation

**Usage:**
```bash
export NODE_COUNT=4
./initialize-dryrun-network.sh
```

**Prerequisites:**
- kubectl configured with access to the AKS cluster
- Port 9944 available for port-forwarding
- Node.js and the Creditcoin CLI built and available

### cleanup-dryrun.sh

Manages cleanup of dryrun Azure resources.

**Usage:**

List all dryrun resource groups:
```bash
./cleanup-dryrun.sh list
```

Delete a specific resource group:
```bash
./cleanup-dryrun.sh delete <resource-group-name>
```

Delete all dryrun resource groups:
```bash
./cleanup-dryrun.sh delete-all
```

Check deletion status:
```bash
./cleanup-dryrun.sh status
```

**Prerequisites:**
- Azure CLI installed and configured
- Appropriate permissions to delete resource groups

## Environment Variables

### initialize-dryrun-network.sh

- `NODE_COUNT`: Number of validator nodes to initialize (default: 4)

## Notes

- The initialization script expects well-known development accounts (Alice, Bob, Charlie, Dave, Eve) to be pre-funded in the genesis configuration
- All validators are configured with a 1% commission rate
- The scripts use colored output for better readability
- Port forwarding is automatically cleaned up after initialization

## Troubleshooting

If the initialization fails:

1. Check that all nodes are running:
   ```bash
   kubectl get pods -n creditcoin-dryrun
   ```

2. Check node logs:
   ```bash
   kubectl logs -n creditcoin-dryrun creditcoin-node-0-0
   ```

3. Verify port forwarding:
   ```bash
   kubectl port-forward -n creditcoin-dryrun svc/creditcoin-node-0 9944:9944
   ```

4. Test connectivity:
   ```bash
   cd cli
   node dist/cli.js status --url ws://localhost:9944
   ```

If cleanup fails:

1. Check Azure CLI authentication:
   ```bash
   az account show
   ```

2. List resource groups manually:
   ```bash
   az group list --query "[?tags.purpose=='dryrun']"
   ```

3. Force delete if needed:
   ```bash
   az group delete --name <resource-group> --yes --no-wait
   ```
