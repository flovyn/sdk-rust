#!/bin/bash
# Run Stateright model checking tests
#
# Usage:
#   ./bin/dev/run-model-check.sh              # Run all model tests
#   ./bin/dev/run-model-check.sh sequence     # Run tests matching 'sequence'
#   ./bin/dev/run-model-check.sh join_all     # Run tests matching 'join_all'

set -e

FILTER="${1:-}"

echo "=== Running Model Checking Tests ==="
echo ""

if [ -n "$FILTER" ]; then
    echo "Filter: $FILTER"
    echo ""
    cargo test --test model -p flovyn-sdk -- --nocapture "$FILTER"
else
    cargo test --test model -p flovyn-sdk -- --nocapture
fi

echo ""
echo "=== Model Checking Complete ==="
