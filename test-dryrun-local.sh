#!/bin/bash
set -euo pipefail

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}Local Dryrun Network Testing${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Test environment variables
echo -e "${BLUE}[1/5] Testing environment variable substitution...${NC}"
export IMAGE_TAG="test-local-$(date +%s)"
export CHAINSPEC="devnetSpecRaw.json"
export NODE_COUNT=4
export NAMESPACE="dryrun-test-local"

echo "  IMAGE_TAG: $IMAGE_TAG"
echo "  CHAINSPEC: $CHAINSPEC"
echo "  NODE_COUNT: $NODE_COUNT"
echo "  NAMESPACE: $NAMESPACE"
echo -e "${GREEN}✓ Variables set${NC}"
echo ""

# Test YAML rendering
echo -e "${BLUE}[2/5] Rendering Kubernetes manifests...${NC}"
envsubst < .github/k8s/dryrun-deployment.yaml > /tmp/dryrun-deployment-test.yaml
echo -e "${GREEN}✓ Manifests rendered to /tmp/dryrun-deployment-test.yaml${NC}"
echo ""

# Test YAML validation (requires kubectl)
echo -e "${BLUE}[3/5] Validating Kubernetes manifests...${NC}"
if kubectl apply --dry-run=client -f /tmp/dryrun-deployment-test.yaml > /dev/null 2>&1; then
    echo -e "${GREEN}✓ Manifests are valid${NC}"
else
    echo -e "${RED}✗ Manifest validation failed${NC}"
    exit 1
fi
echo ""

# Check namespace substitution
echo -e "${BLUE}[4/5] Verifying namespace substitution...${NC}"
if grep -q "namespace: $NAMESPACE" /tmp/dryrun-deployment-test.yaml; then
    echo -e "${GREEN}✓ Namespace correctly substituted in metadata${NC}"
else
    echo -e "${RED}✗ Namespace substitution failed in metadata${NC}"
    exit 1
fi

if grep -q "creditcoin-node-0.$NAMESPACE.svc.cluster.local" /tmp/dryrun-deployment-test.yaml; then
    echo -e "${GREEN}✓ Namespace correctly substituted in bootnode DNS${NC}"
else
    echo -e "${RED}✗ Namespace substitution failed in bootnode DNS${NC}"
    exit 1
fi
echo ""

# Check image tag substitution
echo -e "${BLUE}[5/5] Verifying image tag substitution...${NC}"
if grep -q "gluwa/creditcoin3:$IMAGE_TAG" /tmp/dryrun-deployment-test.yaml; then
    echo -e "${GREEN}✓ Image tag correctly substituted${NC}"
else
    echo -e "${RED}✗ Image tag substitution failed${NC}"
    exit 1
fi
echo ""

# Summary
echo -e "${BLUE}========================================${NC}"
echo -e "${GREEN}All tests passed! ✓${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo "Next steps:"
echo "1. Review rendered manifest: cat /tmp/dryrun-deployment-test.yaml"
echo "2. Test Docker build: docker build -t gluwa/creditcoin3:$IMAGE_TAG --build-arg='BUILD_ARGS=--features fast-runtime,devnet' ."
echo "3. Deploy to local cluster: kubectl create namespace $NAMESPACE && kubectl apply -f /tmp/dryrun-deployment-test.yaml"
echo "4. Test initialization: export NAMESPACE=$NAMESPACE && export NODE_COUNT=$NODE_COUNT && .github/scripts/initialize-dryrun-network.sh"
echo ""
