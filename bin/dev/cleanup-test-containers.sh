#!/bin/bash
# Cleanup orphaned test containers from E2E tests
#
# Usage:
#   ./bin/dev/cleanup-test-containers.sh        # Stop and remove all flovyn test containers
#   ./bin/dev/cleanup-test-containers.sh -l     # List containers only (dry run)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

LIST_ONLY=false

while getopts "l" opt; do
    case $opt in
        l)
            LIST_ONLY=true
            ;;
        *)
            echo "Usage: $0 [-l]"
            echo "  -l  List containers only (dry run)"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}=== Flovyn SDK Test Container Cleanup ===${NC}"
echo

# Find containers by label (preferred method)
LABELED_CONTAINERS=$(docker ps -q --filter "label=flovyn-test=true" 2>/dev/null || true)

# Find containers by image name (fallback for old containers without labels)
POSTGRES_CONTAINERS=$(docker ps -q --filter "ancestor=postgres:16-alpine" 2>/dev/null || true)
NATS_CONTAINERS=$(docker ps -q --filter "ancestor=nats:latest" 2>/dev/null || true)
SERVER_CONTAINERS=$(docker ps -q --filter "ancestor=flovyn-server-test:latest" 2>/dev/null || true)

# Combine and dedupe
ALL_CONTAINERS=$(echo -e "$LABELED_CONTAINERS\n$POSTGRES_CONTAINERS\n$NATS_CONTAINERS\n$SERVER_CONTAINERS" | sort -u | grep -v '^$' || true)

# Count containers
CONTAINER_COUNT=$(echo "$ALL_CONTAINERS" | grep -c . || echo 0)

if [ "$CONTAINER_COUNT" -eq 0 ]; then
    echo -e "${GREEN}No test containers found.${NC}"
else
    echo -e "Found ${YELLOW}$CONTAINER_COUNT${NC} test container(s):"
    echo

    # List containers with details
    for container in $ALL_CONTAINERS; do
        INFO=$(docker inspect --format '{{.Name}} ({{.Config.Image}}) - Started: {{.State.StartedAt}}' "$container" 2>/dev/null || echo "$container")
        echo "  - $INFO"
    done
    echo

    if [ "$LIST_ONLY" = true ]; then
        echo -e "${YELLOW}Dry run - no containers stopped.${NC}"
    else
        echo -e "${RED}Stopping containers...${NC}"
        for container in $ALL_CONTAINERS; do
            echo -n "  Stopping $container... "
            docker stop "$container" >/dev/null 2>&1 && echo -e "${GREEN}done${NC}" || echo -e "${RED}failed${NC}"
        done

        echo
        echo -e "${RED}Removing containers...${NC}"
        for container in $ALL_CONTAINERS; do
            echo -n "  Removing $container... "
            docker rm "$container" >/dev/null 2>&1 && echo -e "${GREEN}done${NC}" || echo -e "${RED}failed${NC}"
        done

        echo
        echo -e "${GREEN}Container cleanup complete.${NC}"
    fi
fi

echo
echo -e "${GREEN}Cleanup finished.${NC}"
