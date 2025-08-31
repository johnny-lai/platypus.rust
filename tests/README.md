# Integration Tests for Platypus

This directory contains comprehensive integration tests for the platypus caching system, using the demo setup as a foundation.

## Test Files

1. **`simple_integration_test.rs`** - ✅ **Working** - Lightweight tests that verify basic platypus functionality
2. **`integration_test.rs`** - 🚧 **In Progress** - Full three-tier architecture tests with proxy support

## Working Tests

### Simple Integration Tests
These tests use the memcached built by the demo system but test platypus directly:

```bash
# Run the working simple integration tests
cargo test -p integration-tests

# With output
cargo test -p integration-tests -- --nocapture
```

**What they test:**
- ✅ Platypus binary can start and respond to help
- ✅ Platypus server can fetch values using default patterns (`test_*` keys)
- ✅ Basic memcached connectivity and functionality
- ✅ Proper value format from platypus (`"test {key} at {timestamp}"`)

### Example Output
```
✅ Platypus binary help test passed
✅ Direct platypus test passed: test test_integration_instance at Instant { tv_sec: 831618, tv_nsec: 671321166 }
✅ Warm cache test passed
```

## Full Integration Tests (In Development)

### Architecture Tested
```
Client → memcached-proxy (port) → warm cache (port)
                |                        ^
                \                        |
                 \- on miss → platypus --/
                            (port)
```

### Test Scenarios Implemented
1. **Cache Miss Flow**: Complete flow when key doesn't exist in warm cache
2. **Cache Hit Flow**: Warm cache serving cached values directly
3. **Background Refresh**: Platypus refreshes values before TTL expires

### Current Status
The full integration tests build correctly but have timing/startup issues. The services (proxy memcached, warm cache, platypus) may not be starting in the correct order or configuration.

## Requirements

- **Rust toolchain**
- **libevent** (install via `brew install libevent` on macOS)
- **Make** (for building memcached with proxy support)

## Manual Demo Testing

To verify the full system works, you can run the demo manually:

```bash
# Start the full demo setup
cd examples/demo
./server.sh

# In another terminal, test the system
echo "get test_integration_key" | nc localhost 21211
```

## Key Insights

1. **Memcached with Proxy Support**: Regular memcached doesn't support proxy functionality. The demo downloads and builds memcached-1.6.39 with `--enable-proxy` flag.

2. **Binary Protocol**: All tests use binary memcached protocol by default as requested.

3. **Dynamic Port Management**: Tests use port ranges 31000+ to avoid conflicts with running services.

4. **Pattern Matching**: Platypus server has default routes for `test_(?<instance>.*)` and `other_(?<instance>.*)` patterns.

## Future Improvements

- Fix timing issues in full integration tests
- Add more comprehensive error handling
- Add tests for different platypus configurations
- Add tests for concurrent access scenarios
