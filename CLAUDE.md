 # CLAUDE.md
 
 This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.
 
 ## Commands
 
 ### Build
 ```bash
 cargo build
 ```
 
 ### Run Tests
 ```bash
 cargo test
 ```
 
 ### Run Server
 ```bash
 cargo run --bin server
 ```
 
 ### Available Command-Line Options
 The server binary supports:
 - `--bind <ADDRESS>` - TCP address to bind to (e.g., "127.0.0.1:11212")
 - `--unix-socket <PATH>` - Unix socket path to bind to (e.g., "/tmp/platypus.sock")
 - `--target <TARGET>` - Target memcached server (default: "memcache://127.0.0.1:11213")
 
 ## Architecture
 
 ### Project Structure
 This is a Rust workspace with two main components:
 - `platypus/` - Core library crate containing the platypus functionality
 - `server/` - Binary crate that runs the platypus server
 
 ### Core Components
 
 **platypus Library (`platypus/src/`)**:
 - `lib.rs` - Main library exports and the `AsyncGetter` trait definition
 - `server.rs` - TCP/Unix socket server that handles memcached protocol connections
 - `service.rs` - Tower service implementation that processes memcached commands
 - `monitor.rs` - Background task system for periodic value fetching and caching
 - `writer.rs` - Handles writing values to target memcached instances
 - `protocol/` - Memcached protocol parsing (binary and text formats)
 
 **Key Architecture Patterns**:
 - Built on Tower service architecture for composable middleware
 - Supports both TCP and Unix socket connections
 - Implements memcached protocol (both text and binary)
 - Background monitoring system with configurable TTLs and intervals
 - Async getter pattern for fetching values from external sources
 
 ### How Platypus Works
 Platypus acts as a memcached proxy that:
 1. Receives memcached GET requests
 2. On cache miss, fetches values using an async getter function
 3. Writes results to a target memcached server with TTL
 4. Runs background monitoring tasks to refresh values before they expire
 5. Supports graceful shutdown via SIGINT/SIGTERM
 
 ### Key Traits and Types
 - `AsyncGetter` - Trait for async functions that fetch values by key
 - `Service<F>` - Main service that processes memcached commands
 - `MonitorTasks<V>` - Manages background refresh tasks
 - `Server` - Handles network connections and protocol parsing
 - `Writer<V>` - Abstracts writing to target memcached servers
 
 ### Testing
 Tests are included in each module and can be run with `cargo test`. The codebase includes comprehensive unit tests for protocol parsing, service logic, and server functionality.
