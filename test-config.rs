// Simple test to verify environment variable override functionality
use std::env;

fn main() {
    // Set environment variables
    env::set_var("PLATYPUS_SERVER_PREFIX", "env-override/");
    env::set_var("PLATYPUS_SERVER_TARGET_HOST", "redis");
    env::set_var("PLATYPUS_SERVER_TARGET_PORT", "6379");

    // Load config from our test TOML file
    match server::config::Config::from_file("test-env-config.toml") {
        Ok(config) => {
            println!("Config loaded successfully!");

            if let Some(server_config) = &config.server {
                println!("Server prefix: {:?}", server_config.prefix);

                if let Some(target_config) = &server_config.target {
                    println!("Target config: {:?}", target_config);
                    println!("Target URL: {}", target_config.to_url());
                }
            }

            println!("Full config: {:#?}", config);
        }
        Err(e) => {
            eprintln!("Error loading config: {}", e);
        }
    }
}