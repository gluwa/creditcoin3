# Dryrun Network Deployment

This document describes how to deploy a temporary dryrun blockchain network on Azure Kubernetes Service (AKS) for testing and development purposes.

## Overview

The dryrun network deployment workflow automatically provisions an Azure Kubernetes cluster, deploys 4-5 Creditcoin blockchain validator nodes, and initializes the network with funded validators ready to produce blocks.

## Features

- Automated Azure AKS cluster provisioning
- Configurable number of validator nodes (default: 4)
- Environment-specific chainspec selection (dev/test/main)
- Automatic validator setup and key rotation
- Manual cleanup for cost control
- Comprehensive logging and status reporting

## Triggering a Dryrun Deployment

### Method 1: Git Tag

Push a tag with the format `dryrun-{environment}`:

```bash
# For devnet
git tag dryrun-dev
git push origin dryrun-dev

# For testnet
git tag dryrun-test
git push origin dryrun-test

# For mainnet
git tag dryrun-main
git push origin dryrun-main
```

The workflow will automatically:
1. Detect the environment from the tag name
2. Use the corresponding chainspec (devnetSpecRaw.json, testnetSpecRaw.json, or mainnetSpecRaw.json)
3. Build a Docker image with appropriate build features
4. Deploy 4 validator nodes

### Method 2: Manual Workflow Dispatch

You can also manually trigger the workflow from GitHub Actions:

1. Go to Actions → Deploy Dryrun Network
2. Click "Run workflow"
3. Select the environment (dev/test/main)
4. Specify the number of validator nodes (4-5)
5. Click "Run workflow"

## Architecture

### Infrastructure

- **Azure Resource Group**: Unique per deployment (creditcoin-dryrun-{run_id})
- **AKS Cluster**: 2 Standard_D4s_v3 nodes
- **Storage**: 50Gi persistent volumes per blockchain node
- **Networking**: Azure CNI with ClusterIP services

### Blockchain Nodes

The deployment creates the following validator nodes:

| Node | Account | Role | Ports |
|------|---------|------|-------|
| node-0 | Alice | Bootnode + Validator | 9944, 9933, 30333, 9615 |
| node-1 | Bob | Validator | 9944, 9933, 30333, 9615 |
| node-2 | Charlie | Validator | 9944, 9933, 30333, 9615 |
| node-3 | Dave | Validator | 9944, 9933, 30333, 9615 |
| node-4 | Eve | Validator (optional) | 9944, 9933, 30333, 9615 |

Each node:
- Runs as a Kubernetes StatefulSet
- Has its own persistent volume for blockchain data
- Exposes RPC, WebSocket, P2P, and Prometheus ports
- Uses well-known development accounts (Alice, Bob, etc.)

### Network Initialization

After deployment, the workflow automatically:

1. **Bonds tokens**: Each validator bonds 10,000 tokens
2. **Rotates keys**: Generates new session keys for each node
3. **Sets keys**: Associates session keys with validator accounts
4. **Starts validation**: Signals intention to validate with 1% commission

## Accessing the Network

After successful deployment, you'll receive connection information in the workflow output and as an artifact.

### Connect to the Bootnode

```bash
# Get AKS credentials
az aks get-credentials \
  --resource-group <resource-group-name> \
  --name <cluster-name>

# Port forward to access the bootnode
kubectl port-forward -n creditcoin-dryrun svc/creditcoin-node-0 9944:9944

# Connect via WebSocket
ws://localhost:9944
```

### View Node Logs

```bash
# All nodes
kubectl logs -n creditcoin-dryrun -l app=creditcoin-node --tail=100 -f

# Specific node
kubectl logs -n creditcoin-dryrun creditcoin-node-0-0 -f
```

### Check Node Status

```bash
# Get pods
kubectl get pods -n creditcoin-dryrun -o wide

# Get services
kubectl get services -n creditcoin-dryrun

# Check validator status
cd cli
node dist/cli.js status --url ws://localhost:9944
```

## Monitoring

### Kubernetes Dashboard

```bash
# Create proxy
kubectl proxy

# Access dashboard
http://localhost:8001/api/v1/namespaces/kubernetes-dashboard/services/https:kubernetes-dashboard:/proxy/
```

### Prometheus Metrics

Each node exposes Prometheus metrics on port 9615:

```bash
kubectl port-forward -n creditcoin-dryrun svc/creditcoin-node-0 9615:9615

# Access metrics
curl http://localhost:9615/metrics
```

## Cleanup

### Manual Cleanup (Recommended)

To delete the dryrun network and all associated resources:

```bash
# List all dryrun deployments
.github/scripts/cleanup-dryrun.sh list

# Delete a specific deployment
.github/scripts/cleanup-dryrun.sh delete <resource-group-name>

# Or delete all dryrun deployments
.github/scripts/cleanup-dryrun.sh delete-all

# Check deletion status
.github/scripts/cleanup-dryrun.sh status
```

Alternatively, use Azure CLI directly:

```bash
az group delete --name <resource-group-name> --yes --no-wait
```

### Automatic Cleanup

Currently, the workflow does not include automatic cleanup. This is intentional to:
- Allow extended testing periods
- Prevent accidental deletion of active test networks
- Give you control over cost management

## Cost Considerations

The dryrun network uses the following Azure resources:

- **AKS Cluster**: 2 × Standard_D4s_v3 VMs (~$0.24/hour each)
- **Storage**: 5 × 50Gi managed disks (~$0.005/GB/month)
- **Network**: Standard Azure networking costs

**Estimated cost**: ~$0.50-0.60/hour or ~$12-15/day

**Important**: Always delete resource groups when not in use to avoid unnecessary charges.

## Troubleshooting

### Deployment Fails

1. Check workflow logs in GitHub Actions
2. Verify Azure credentials are correctly configured
3. Ensure sufficient Azure quota for the resources

### Nodes Not Starting

```bash
# Check pod status
kubectl get pods -n creditcoin-dryrun

# Check pod events
kubectl describe pod -n creditcoin-dryrun <pod-name>

# Check logs
kubectl logs -n creditcoin-dryrun <pod-name>
```

### Initialization Fails

1. Check that all pods are in Running state
2. Verify port forwarding is working
3. Check CLI can connect to nodes
4. Review initialization script logs in workflow output

### Network Not Producing Blocks

1. Check validator status: `kubectl logs -n creditcoin-dryrun -l app=creditcoin-node --tail=50`
2. Verify validators are active (wait 1-2 eras after initialization)
3. Check session keys are correctly set
4. Ensure nodes can communicate (check P2P connectivity)

## Configuration

### Required GitHub Secrets

- `AZURE_CREDENTIALS`: Azure service principal credentials
- `DOCKER_PUSH_USERNAME`: Docker Hub username
- `DOCKER_PUSH_PASSWORD`: Docker Hub password/token

### Azure Credentials Format

```json
{
  "clientId": "<client-id>",
  "clientSecret": "<client-secret>",
  "subscriptionId": "<subscription-id>",
  "tenantId": "<tenant-id>"
}
```

### Environment Variables

In the workflow:
- `AZURE_LOCATION`: Azure region (default: eastus)
- `NODE_COUNT`: Number of validators (default: 4)

## Advanced Usage

### Custom Chainspec

To use a custom chainspec:

1. Add your chainspec to the `chainspecs/` directory
2. Modify the workflow to reference your chainspec
3. Update the ConfigMap creation in the deployment step

### Different Node Configurations

To customize node arguments:

1. Edit `.github/k8s/dryrun-deployment.yaml`
2. Modify the `args` section for each StatefulSet
3. Adjust resources, storage, or networking as needed

### Add More Validators

To add more than 5 validators:

1. Update `NODE_COUNT` environment variable
2. Add corresponding StatefulSet and Service definitions in the deployment manifest
3. Add account mappings in the initialization script

## Security Considerations

- Nodes run with `--rpc-methods=unsafe` for testing purposes
- Well-known development accounts are used (not secure for production)
- RPC and WebSocket ports are exposed cluster-internally only
- Access requires kubectl access to the cluster

**Warning**: This setup is for testing only. Do not use for production networks.

## Support

For issues or questions:
1. Check troubleshooting section above
2. Review workflow logs in GitHub Actions
3. Check Kubernetes pod logs and events
4. Open an issue in the repository

## References

- [Azure Kubernetes Service Documentation](https://docs.microsoft.com/en-us/azure/aks/)
- [Kubernetes Documentation](https://kubernetes.io/docs/)
- [Substrate Documentation](https://docs.substrate.io/)
