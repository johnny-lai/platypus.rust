#!/usr/bin/env bash
set -e

# Check service is running
check_service_running() {
    local pid=$1
    local name=$2
    if ! kill -0 $pid 2>/dev/null; then
        echo "Error: $name process (PID $pid) is not running"
        return 1
    fi
    echo "$name process is running (PID $pid)"
    return 0
}

# cleanup on exit
cleanup() {
    echo "Cleaning up services..."
    kill $PROXY_PID $WARM_PID $COLD_PID 2>/dev/null || true
}

MEMCACHED=./tmp/memcached-1.6.39/memcached
PROXY_PORT=21211
export WARM_PORT=21212
export COLD_PORT=21213

## Auto-build tmp/memcached if needed
if [ ! -x $MEMCACHED ]; then
    echo "Building $MEMCACHED ..."
    make $MEMCACHED
fi

## Other services for the demo

## Proxy configuration
# 1. This is what other services will connect to
$MEMCACHED -l 127.0.0.1 -p $PROXY_PORT -o proxy_config=routelib,proxy_arg=config.lua &
PROXY_PID=$!
echo "Started proxy on $PROXY_PORT with pid=$PROXY_PID"

# 2. This is the memcache holding the warm values
$MEMCACHED -l 127.0.0.1 -p $WARM_PORT  &
WARM_PID=$!
echo "Started warm cache on $WARM_PORT with pid=$WARM_PID"

# cleanup on exit
trap cleanup EXIT

# Check is all dependent services have started
sleep 0.25
echo "Checking if dependent services are running..."
check_service_running $PROXY_PID "Proxy" || exit 1
check_service_running $WARM_PID "Warm cache" || exit 1
echo "All dependent services are running!"

# 3. If an entry is not in the warm cache, then it get routed to playtpus.
#    This is last. When this quits, all the other services will be killed.
cargo run --bin server -- \
    -b 127.0.0.1:$COLD_PORT \
    -t memcache://127.0.0.1:$WARM_PORT
COLD_PID=$!
