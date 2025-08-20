# Platypus Demo Setup

This demo demonstrates a complete three-tier caching setup using platypus as a memcached proxy with background fetching capabilities.

## Architecture

```
Client --> memcached-proxy (port 21211) --> warm cache (port 21213)
                    |                              ^
                    \                              |
                     \- on miss --> platypus ----/
                                   (port 21212)
```

### Components

1. **Memcached Proxy** (port 21211): Routes requests based on configuration
2. **Warm Cache** (port 21213): Holds cached values with TTL
3. **Platypus Server** (port 21212): Fetches values on demand and populates warm cache

## Setup Instructions

### Prerequisites

- Rust toolchain
- libevent (install via `brew install libevent` on macOS)

### Build and Run

1. **Start the demo:**
   ```bash
   ./server.sh
   ```

This will:
- Build memcached with proxy support
- Start all three services:
  - Memcached proxy on port 21211
  - Warm cache on port 21213
  - Platypus server on port 21212

### Testing

Connect to the proxy and test caching:

```bash
# Connect to the proxy and read a key
% echo "get test_key" | nc localhost 21211
VALUE test_key 0 64
value_for_test_key Instant { tv_sec: 322629, tv_nsec: 98373083 }
END
```

On first request:
1. Proxy routes to warm cache (miss)
2. Request falls through to platypus
3. Platypus fetches value and stores in warm cache
4. Subsequent requests hit warm cache directly

## Configuration Files

- `config.lua`: Memcached proxy routing configuration
- `config.toml`: Platypus server configuration (example for advanced setups)

## Cleanup

The `./server.sh` script handles cleanup automatically. Press Ctrl+C to stop all services.
