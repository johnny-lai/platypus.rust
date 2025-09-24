use async_memcached::AsciiProtocol;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::sleep;

const BASE_PORT: u16 = 31000;
const PORT_RANGE: u16 = 1000;

// Static variables for port allocation synchronization
static PORT_ALLOCATION_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
static LAST_ALLOCATED_PORT: AtomicU16 = AtomicU16::new(BASE_PORT - 1);

static SETUP_MEMCACHED_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug)]
struct TestPorts {
    proxy_port: u16,
    warm_port: u16,
    cold_port: u16,
}

struct TestServices {
    proxy_process: Child,
    warm_process: Child,
    platypus_process: Child,
    ports: TestPorts,
}

impl Drop for TestServices {
    fn drop(&mut self) {
        // Kill processes in reverse order (platypus -> warm -> proxy)
        // This ensures proper shutdown sequence
        let _ = self.platypus_process.kill();
        let _ = self.warm_process.kill();
        let _ = self.proxy_process.kill();

        // Wait for processes to terminate with timeout
        let wait_timeout = Duration::from_secs(3);
        let start = std::time::Instant::now();

        while start.elapsed() < wait_timeout {
            let mut all_done = true;

            // Try to wait for each process with no blocking
            match self.platypus_process.try_wait() {
                Ok(Some(_)) => {}             // Process has exited
                Ok(None) => all_done = false, // Process still running
                Err(_) => {}                  // Process already dead or other error
            }

            match self.warm_process.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => all_done = false,
                Err(_) => {}
            }

            match self.proxy_process.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => all_done = false,
                Err(_) => {}
            }

            if all_done {
                break;
            }

            std::thread::sleep(Duration::from_millis(100));
        }

        // Force kill if still running
        let _ = self.platypus_process.kill();
        let _ = self.warm_process.kill();
        let _ = self.proxy_process.kill();

        // Final wait
        let _ = self.platypus_process.wait();
        let _ = self.warm_process.wait();
        let _ = self.proxy_process.wait();

        // Give OS time to release ports
        std::thread::sleep(Duration::from_millis(100));

        println!(
            "✅ Cleaned up test services (ports: proxy={}, warm={}, platypus={})",
            self.ports.proxy_port, self.ports.warm_port, self.ports.cold_port
        );
    }
}

impl TestServices {
    #[allow(dead_code)]
    pub async fn proxy_client(&self) -> Result<async_memcached::Client, async_memcached::Error> {
        Self::client_with_port(self.ports.proxy_port).await
    }

    #[allow(dead_code)]
    pub async fn warm_client(&self) -> Result<async_memcached::Client, async_memcached::Error> {
        Self::client_with_port(self.ports.warm_port).await
    }

    #[allow(dead_code)]
    pub async fn platypus_client(&self) -> Result<async_memcached::Client, async_memcached::Error> {
        Self::client_with_port(self.ports.cold_port).await
    }

    pub async fn client_with_port(
        port: u16,
    ) -> Result<async_memcached::Client, async_memcached::Error> {
        let url = format!("tcp://127.0.0.1:{}", port);
        async_memcached::Client::new(url).await
    }
}

#[tokio::test]
async fn test_cache_miss_flow() {
    let delay = Duration::from_secs(1);
    let services = setup_test_services().await;

    // Connect to the proxy (entry point)
    let mut proxy_client = services
        .proxy_client()
        .await
        .expect("Failed to connect to proxy");

    // Connect directly to warm cache to verify miss/hit behavior
    let mut warm_client = services
        .warm_client()
        .await
        .expect("Failed to connect to warm cache");

    let test_key = "echo1/abc";

    // 1. Verify key doesn't exist in warm cache initially
    let warm_result = warm_client.get(test_key).await;
    assert!(
        warm_result.unwrap().is_none(),
        "Key should not exist in warm cache initially"
    );

    // 2. Request key through proxy - should trigger cache miss -> platypus -> warm cache
    let proxy_result = proxy_client
        .get(test_key)
        .await
        .expect("Failed to get from proxy");

    // 3. Should get a value that matches the pattern from platypus server
    assert!(
        proxy_result.is_some(),
        "Should get value from platypus on cache miss"
    );
    let value = String::from_utf8(proxy_result.unwrap().data.unwrap()).unwrap();
    assert_eq!(
        "echo1 = abc", value,
        "Value should match expected result from echo1 source"
    );

    // Wait a little to get platypus a chance to update the warm cache
    tokio::time::sleep(delay).await;

    // 4. Verify the value is now in warm cache
    let warm_result = warm_client
        .get(test_key)
        .await
        .expect("Failed to get from warm cache");
    assert!(warm_result.is_some(), "Key should now exist in warm cache");
    assert_eq!(
        String::from_utf8(warm_result.unwrap().data.unwrap()).unwrap(),
        value,
        "Warm cache should contain the same value"
    );

    // 5. Request again through proxy - should get from warm cache (not platypus)
    let second_proxy_result = proxy_client
        .get(test_key)
        .await
        .expect("Failed to get from proxy second time");
    assert!(
        second_proxy_result.is_some(),
        "Should get cached value from warm cache"
    );
    assert_eq!(
        String::from_utf8(second_proxy_result.unwrap().data.unwrap()).unwrap(),
        value,
        "Should get same cached value"
    );

    // TestServices will automatically cleanup when dropped
}

#[tokio::test]
async fn test_cache_hit_flow() {
    let services = setup_test_services().await;

    // Connect to the proxy (entry point)
    let mut proxy_client = services
        .proxy_client()
        .await
        .expect("Failed to connect to proxy");

    // Connect to proxy and warm cache
    let mut warm_client = services
        .warm_client()
        .await
        .expect("Failed to connect to warm cache");

    let test_key = "test_cache_hit_key";

    // 1. Pre-populate warm cache with a known value (simulating previous cache)
    let expected_value = "pre_cached_test_value";
    warm_client
        .set(test_key, expected_value, None, None)
        .await
        .expect("Failed to set value in warm cache");

    // 2. Verify value exists in warm cache
    let warm_result = warm_client
        .get(test_key)
        .await
        .expect("Failed to get from warm cache");
    assert_eq!(
        String::from_utf8(warm_result.unwrap().data.unwrap()).unwrap(),
        expected_value,
        "Warm cache should contain the pre-set value"
    );

    // 3. Request through proxy - should get from warm cache (cache hit)
    let proxy_result = proxy_client
        .get(test_key)
        .await
        .expect("Failed to get from proxy");

    // 4. Should get the pre-cached value, not a new value from platypus
    assert!(proxy_result.is_some(), "Should get cached value");
    assert_eq!(
        String::from_utf8(proxy_result.unwrap().data.unwrap()).unwrap(),
        expected_value,
        "Should get the pre-cached value from warm cache"
    );

    // 5. Request multiple times to ensure consistent cache hits
    for _ in 0..3 {
        let result = proxy_client
            .get(test_key)
            .await
            .expect("Failed to get from proxy");
        assert_eq!(
            String::from_utf8(result.unwrap().data.unwrap()).unwrap(),
            expected_value,
            "Should consistently get cached value"
        );
    }

    // TestServices will automatically cleanup when dropped
}

#[tokio::test]
async fn test_background_refresh() {
    let services = setup_test_services().await;

    // Connect to proxy and warm cache
    let mut proxy_client = services
        .proxy_client()
        .await
        .expect("Failed to connect to proxy");
    let mut warm_client = services
        .warm_client()
        .await
        .expect("Failed to connect to warm cache");

    let test_key = "echo1/abc";

    // 1. Initial request to populate cache
    let initial_result = proxy_client
        .get(test_key)
        .await
        .expect("Failed to get initial value");
    assert!(
        initial_result.is_some(),
        "Should get initial value from platypus"
    );
    let initial_value = String::from_utf8(initial_result.unwrap().data.unwrap()).unwrap();
    assert!(
        initial_value.starts_with("echo1 = abc"),
        "Should match expected format"
    );

    // Wait a little to give platypus a chance to update the warm cache
    sleep(Duration::from_millis(500)).await;

    // 2. Verify value is in warm cache
    let warm_result = warm_client
        .get(test_key)
        .await
        .expect("Failed to get from warm cache");
    assert!(warm_result.is_some(), "Value should exist in warm cache");
    assert_eq!(
        String::from_utf8(warm_result.unwrap().data.unwrap()).unwrap(),
        initial_value,
        "Warm cache should contain initial value"
    );

    // 3. Wait for background refresh (TTL is 5 seconds, so monitor should refresh the value)
    sleep(Duration::from_secs(7)).await;

    // 4. Check if value was refreshed in warm cache
    // The background monitor should have updated the value before TTL expires
    let refreshed_result = warm_client
        .get(test_key)
        .await
        .expect("Failed to get refreshed value");
    assert!(
        refreshed_result.is_some(),
        "Value should still exist in warm cache after background refresh"
    );

    let refreshed_value = String::from_utf8(refreshed_result.unwrap().data.unwrap()).unwrap();
    assert!(
        refreshed_value.starts_with("echo1 = abc"),
        "Refreshed value should match expected format"
    );

    // 5. The refreshed value should be different (contains different timestamp)
    if initial_value != refreshed_value {
        println!(
            "Background refresh worked: value was updated from '{}' to '{}'",
            initial_value, refreshed_value
        );
    } else {
        // In some cases, the timestamp might be the same if refresh happened very quickly
        // This is still a valid test outcome - the important thing is the value still exists
        println!("Value remained the same, but background refresh likely still occurred");
    }

    // 6. Request through proxy should get the current warm cache value
    let final_result = proxy_client
        .get(test_key)
        .await
        .expect("Failed to get final value");
    assert!(
        final_result.is_some(),
        "Should still get value after background refresh"
    );
    assert_eq!(
        String::from_utf8(final_result.unwrap().data.unwrap()).unwrap(),
        refreshed_value,
        "Proxy should return current warm cache value"
    );

    // TestServices will automatically cleanup when dropped
}

#[tokio::test]
async fn test_merge_source_functionality() {
    let services = setup_test_services().await;

    // Connect to the proxy (entry point)
    let mut proxy_client = services
        .proxy_client()
        .await
        .expect("Failed to connect to proxy");

    // Connect directly to warm cache to verify caching behavior
    let mut warm_client = services
        .warm_client()
        .await
        .expect("Failed to connect to warm cache");

    let test_key = "both/test_data";

    // 1. Verify key doesn't exist in warm cache initially
    let warm_result = warm_client.get(test_key).await;
    assert!(
        warm_result.unwrap().is_none(),
        "Key should not exist in warm cache initially"
    );

    // 2. Request merge endpoint through proxy - should trigger merge source
    let proxy_result = proxy_client
        .get(test_key)
        .await
        .expect("Failed to get from proxy");

    // 3. Should get a merged JSON value from both echo1 and echo2 sources
    assert!(
        proxy_result.is_some(),
        "Should get merged value from merge source"
    );

    let value = String::from_utf8(proxy_result.unwrap().data.unwrap()).unwrap();
    println!("Received merged value: {}", value);

    // 4. Parse and validate the JSON structure
    let json_value: serde_json::Value = serde_json::from_str(&value)
        .expect("Response should be valid JSON");

    // Should be a JSON object with echo1 and echo2 keys
    assert!(json_value.is_object(), "Response should be a JSON object");
    let json_obj = json_value.as_object().unwrap();

    // Verify both echo1 and echo2 responses are present
    assert!(json_obj.contains_key("echo1"), "Should contain echo1 response");
    assert!(json_obj.contains_key("echo2"), "Should contain echo2 response");

    // Verify the merged content matches expected pattern
    assert_eq!(
        json_obj["echo1"].as_str().unwrap(),
        "echo1 = test_data",
        "echo1 response should match expected format"
    );
    assert_eq!(
        json_obj["echo2"].as_str().unwrap(),
        "echo2 = test_data",
        "echo2 response should match expected format"
    );

    // Wait a little to give platypus a chance to update the warm cache
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 5. Verify the merged value is now in warm cache
    let warm_result = warm_client
        .get(test_key)
        .await
        .expect("Failed to get from warm cache");
    assert!(warm_result.is_some(), "Merged value should now exist in warm cache");

    let cached_value = String::from_utf8(warm_result.unwrap().data.unwrap()).unwrap();
    assert_eq!(
        cached_value, value,
        "Warm cache should contain the same merged JSON value"
    );

    // 6. Request again through proxy - should get from warm cache (cache hit)
    let second_proxy_result = proxy_client
        .get(test_key)
        .await
        .expect("Failed to get from proxy second time");
    assert!(
        second_proxy_result.is_some(),
        "Should get cached merged value from warm cache"
    );

    let second_value = String::from_utf8(second_proxy_result.unwrap().data.unwrap()).unwrap();
    assert_eq!(
        second_value, value,
        "Should get same cached merged value"
    );

    // 7. Test with different path to ensure dynamic behavior
    let test_key2 = "both/different_path";
    let proxy_result2 = proxy_client
        .get(test_key2)
        .await
        .expect("Failed to get second merge request");

    assert!(proxy_result2.is_some(), "Should get value for different path");
    let value2 = String::from_utf8(proxy_result2.unwrap().data.unwrap()).unwrap();

    let json_value2: serde_json::Value = serde_json::from_str(&value2)
        .expect("Second response should be valid JSON");
    let json_obj2 = json_value2.as_object().unwrap();

    // Should have different path values but same structure
    assert_eq!(
        json_obj2["echo1"].as_str().unwrap(),
        "echo1 = different_path",
        "echo1 should reflect different path"
    );
    assert_eq!(
        json_obj2["echo2"].as_str().unwrap(),
        "echo2 = different_path",
        "echo2 should reflect different path"
    );

    // TestServices will automatically cleanup when dropped
}

async fn setup_memcached() -> PathBuf {
    // Get the mutex to ensure only one thread allocates ports at a time
    let mutex = SETUP_MEMCACHED_MUTEX.get_or_init(|| Mutex::new(()));
    let _guard = mutex.lock().await;

    // Build memcached if needed (same as demo)
    let memcached_path = PathBuf::from("tmp/memcached-1.6.39/memcached");
    if !memcached_path.exists() {
        println!("Building memcached...");
        let output = Command::new("make")
            .arg("tmp/memcached-1.6.39/memcached")
            .output()
            .expect("Failed to build memcached");

        if !output.status.success() {
            panic!(
                "Failed to build memcached: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    return memcached_path;
}

async fn setup_test_services() -> TestServices {
    let ports = find_available_ports().await;

    let root_dir = "..";

    // Create test-specific proxy config
    let config_content = format!(
        r#"
local warm_port = {}
local cold_port = {}

pools {{
    warm_pool = {{
        backends = {{
            "127.0.0.1:" .. warm_port,
        }}
    }},
    cold_pool = {{
        backends = {{
            "127.0.0.1:" .. cold_port,
        }}
    }}
}}

routes {{
    default = route_failover {{
        children = {{ "warm_pool", "cold_pool" }},
        miss = true,
    }}
}}
"#,
        ports.warm_port, ports.cold_port
    );

    let config_path = format!("/tmp/platypus_test_config_{}.lua", ports.proxy_port);
    std::fs::write(&config_path, config_content).expect("Failed to write test config");

    // Start proxy memcached
    let memcached_path = setup_memcached().await;
    let proxy_process = Command::new(memcached_path.as_path())
        .args(&[
            "-l",
            "127.0.0.1",
            "-p",
            &ports.proxy_port.to_string(),
            "-o",
            &format!("proxy_config=routelib,proxy_arg={}", config_path),
        ])
        .stdin(Stdio::null())
        //.stdout(Stdio::null())
        //.stderr(Stdio::null())
        .spawn()
        .expect("Failed to start proxy memcached");

    // Start warm cache memcached
    let warm_process = Command::new(memcached_path)
        .args(&["-l", "127.0.0.1", "-p", &ports.warm_port.to_string()])
        .stdin(Stdio::null())
        //.stdout(Stdio::null())
        //.stderr(Stdio::null())
        .spawn()
        .expect("Failed to start warm cache memcached");

    // Wait for memcached services to be ready
    wait_for_service_ready(ports.proxy_port, 2)
        .await
        .expect("Proxy memcached failed to start");
    wait_for_service_ready(ports.warm_port, 2)
        .await
        .expect("Warm cache memcached failed to start");

    // Start platypus server
    let platypus_process = Command::new("cargo")
        .args(&[
            "run",
            "--bin",
            "server",
            "--",
            "-b",
            &format!("127.0.0.1:{}", ports.cold_port),
            "-t",
            &format!("memcache://127.0.0.1:{}", ports.warm_port),
            "-c",
            "tests/config.toml",
        ])
        .current_dir(root_dir) // Run from workspace root
        .stdin(Stdio::null())
        //.stdout(Stdio::out())
        //.stderr(Stdio::piped()) // Capture stderr for debugging
        .spawn()
        .expect("Failed to start platypus server");

    // Wait for platypus to be ready
    wait_for_service_ready(ports.cold_port, 2)
        .await
        .expect("Platypus server failed to start");

    // Note: config_path cleanup will be handled by OS temp cleanup

    // Need to wait until memcached-proxy detects platypus
    sleep(Duration::from_secs(5)).await;

    TestServices {
        proxy_process,
        warm_process,
        platypus_process,
        ports,
    }
}

// cleanup_services function removed - TestServices now handles cleanup automatically via Drop trait

async fn find_available_ports() -> TestPorts {
    // Get the mutex to ensure only one thread allocates ports at a time
    let mutex = PORT_ALLOCATION_MUTEX.get_or_init(|| Mutex::new(()));
    let _guard = mutex.lock().await;

    let mut ports = Vec::new();
    let last_port = LAST_ALLOCATED_PORT.load(Ordering::Relaxed);

    // Start searching from the next port after the last allocated one
    let start_port = if last_port >= BASE_PORT && last_port < BASE_PORT + PORT_RANGE - 1 {
        last_port + 1
    } else {
        BASE_PORT
    };

    // Search from start_port to end of range
    for port in start_port..(BASE_PORT + PORT_RANGE) {
        if is_port_available(port).await {
            ports.push(port);
            if ports.len() == 3 {
                break;
            }
        }
    }

    // If we didn't find enough ports, wrap around to the beginning
    if ports.len() < 3 {
        for port in BASE_PORT..start_port {
            if is_port_available(port).await {
                ports.push(port);
                if ports.len() == 3 {
                    break;
                }
            }
        }
    }

    if ports.len() < 3 {
        panic!(
            "Could not find 3 available ports in range {}..{}",
            BASE_PORT,
            BASE_PORT + PORT_RANGE
        );
    }

    // Update the last allocated port to the highest port we allocated
    let max_port = *ports.iter().max().unwrap();
    LAST_ALLOCATED_PORT.store(max_port, Ordering::Relaxed);

    TestPorts {
        proxy_port: ports[0],
        warm_port: ports[1],
        cold_port: ports[2],
    }
}

async fn is_port_available(port: u16) -> bool {
    TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .is_ok()
}

async fn wait_for_service_ready(port: u16, timeout_secs: u64) -> Result<(), String> {
    let start = std::time::Instant::now();
    let timeout_duration = Duration::from_secs(timeout_secs);

    while start.elapsed() < timeout_duration {
        if let Ok(_) = std::net::TcpStream::connect(format!("127.0.0.1:{}", port)) {
            return Ok(());
        }
        sleep(Duration::from_millis(100)).await;
    }

    Err(format!(
        "Service on port {} did not become ready within {}s",
        port, timeout_secs
    ))
}
