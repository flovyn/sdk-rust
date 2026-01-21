#!/bin/bash
set -e

# Flovyn SDK Examples - Docker Entrypoint
#
# Select which example to run via EXAMPLE environment variable.
# Available: hello-world, ecommerce, data-pipeline, patterns, standalone-tasks

EXAMPLE="${EXAMPLE:-hello-world}"

case "$EXAMPLE" in
    hello-world)
        exec /app/bin/hello-world "$@"
        ;;
    ecommerce)
        exec /app/bin/ecommerce "$@"
        ;;
    data-pipeline)
        exec /app/bin/data-pipeline "$@"
        ;;
    patterns)
        exec /app/bin/patterns "$@"
        ;;
    standalone-tasks)
        exec /app/bin/standalone-tasks "$@"
        ;;
    *)
        echo "Unknown example: $EXAMPLE"
        echo "Available examples: hello-world, ecommerce, data-pipeline, patterns, standalone-tasks"
        exit 1
        ;;
esac
