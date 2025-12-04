#!/bin/bash
# Run MCP tests with isolated test directory

TEST_DIR=$(mktemp -d)
echo "Using test directory: $TEST_DIR"

export JIT_DATA_DIR="$TEST_DIR/.jit"

npm test

EXIT_CODE=$?

# Cleanup
rm -rf "$TEST_DIR"

exit $EXIT_CODE
