use async_memcached::AsciiProtocol;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Once, OnceLock};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::sleep;

static INIT: Once = Once::new();

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

pub struct TestServices {
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
    pub async fn setup() -> TestServices {
        let ports = find_available_ports().await;

        let root_dir = "..";

        // Ensure binary is built
        Self::ensure_binary_built(root_dir);

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
        let platypus_process = Command::new("target/debug/server")
            .args(&[
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
            .env("AWS_ENDPOINT_URL", "http://localhost:4566")
            .env("AWS_ACCESS_KEY_ID", "test")
            .env("AWS_SECRET_ACCESS_KEY", "test")
            .env("AWS_DEFAULT_REGION", "us-east-1")
            .spawn()
            .expect("Failed to start platypus server");

        // Wait for platypus to be ready
        wait_for_service_ready(ports.cold_port, 10)
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

    fn ensure_binary_built(dir: &str) {
        INIT.call_once(|| {
            let output = Command::new("cargo")
                .args(&["build", "--bin", "server"])
                .current_dir(dir)
                .output()
                .expect("Failed to execute cargo build");

            if !output.status.success() {
                panic!(
                    "Failed to build binary: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        });
    }

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

    pub async fn proxy_get_string(&self, key: &str) -> anyhow::Result<String> {
        let proxy_result = self.proxy_client().await?.get(key).await?;

        match proxy_result {
            Some(value) => {
                let value = String::from_utf8(value.data.unwrap()).unwrap();
                Ok(value)
            }
            None => Err(anyhow::anyhow!("None")),
        }
    }
}

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
