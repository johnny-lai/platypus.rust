# platypus

```
          __,---"""----..,,_
  _,...--'                o `====
 `-___..3>/...____...--3>/''
```

Periodically fetches answers in the background into memcached
* On `get`, platypus will start fetching in the background and updating the "target" memcache, with a TTL
* After a certain time the TTL on the target will expire, and then new `get` requests will come back to platypus.
  This is an indication that the key is still needed, and platypus will extend the background fetch time.

```
Client --> McRouter/memcached-proxy ---> target memcache
                        |                           ^
                        \                           |
                         \- on miss ---> platypus --/
                                          |  ^
                                          |  |
                                          v  |
                                      actual service
```

## Example

Start a platypus server that:
* Listens on port `11212`.
* Any get returns `format!("value_for_{}", key)`
* Result is written to memcached on port `11213`

```
let router = Router::new()
  .route(
      "test_(?<instance>.*)",
      source(|key| async move {
          Some(format!("test {key} at {:?}", Instant::now()).to_string())
      })
      .with_ttl(Duration::from_secs(5))
      .with_expiry(Duration::from_secs(30))
      .with_box(),
  )
  .route(
      "other_(?<instance>.*)",
      source(|key| async move {
          Some(format!("other {key} at {:?}", Instant::now()).to_string())
      })
      .with_ttl(Duration::from_secs(5))
      .with_expiry(Duration::from_secs(30))
      .with_box(),
  )
Server::bind("127.0.0.1:11212")
    .router(router)
    .target("memcache://127.0.0.1:11213")
    .run()
    .await
```

## Demo

See [examples/demo/README.md](examples/demo/README.md)

## Compatibility with Rails.cache

To allow the Rails.cache to be able to read raw values, you can try configuring it as follows:

```
config.cache_store = :mem_cache_store, { serializer: :passthrough, raw: true }
```

* `raw: true` is to tell `MemCacheStore` not to expect the result to be wrapped in an `Entry` object.
* `serializer: :passthrough` is to tell `MemCacheStore` not to serialize. Rails can serialize with different
  formats like `:marshal_6_1`, `:marshal_7_0`, `:marshal_7_1`, `:message_pack`.

## Configuration

Platypus can be configured using a TOML configuration file. This allows you to define sources, routes, and server settings declaratively.

### Configuration File Structure

```toml
[server]
target = { host = "localhost", port = 11213 }
prefix = "optional-prefix/"

[routes.instance]
routes = [
  { match = "^pattern/(?<name>.+)", to = "source_name" },
]

[source.source_name]
type = "source_type"
# source-specific configuration...
```

### Server Configuration

The `[server]` section configures the target memcached server and optional settings:

```toml
[server]
target = { host = "localhost", port = 11213 }  # Target memcached server
prefix = "my-app/"                             # Optional key prefix
```

- `target` - Target memcached server configuration
  - `host` - Hostname or IP address
  - `port` - Port number
- `prefix` - Optional prefix added to all keys (useful for namespacing)

### Routes Configuration

Routes define URL patterns and map them to data sources. Each route group is defined under `[routes.name]`:

```toml
[routes.api]
routes = [
  { match = "^config/(?<instance>[^/]+)$", to = "app_config" },
  { match = "^secret/(?<instance>[^/]+)$", to = "app_secret" },
  { match = "^both/(?<instance>[^/]+)$", to = "merged_data" },
]
```

- `match` - Regular expression pattern with named capture groups
- `to` - Name of the source to use for this route

Named capture groups (like `(?<instance>.+)`) become variables available to sources.

### Source Types

Sources define how to fetch data. Each source is configured under `[source.name]`:

#### Echo Source

Returns a templated string response:

```toml
[source.greeting]
type = "echo"
template = "Hello {name}!"
```

- `template` - Template string with `{variable}` placeholders

#### HTTP Source

Fetches data from HTTP endpoints:

```toml
[source.api_data]
type = "http"
url = "https://api.example.com/data/%(instance)"
method = "GET"                                    # Optional: GET, POST, PUT, DELETE
headers = { "Authorization" = "Bearer token" }    # Optional headers
query = { "format" = "json" }                    # Optional query parameters
ttl = "30s"                                      # Cache TTL
expiry = "300s"                                  # Background refresh duration
```

- `url` - URL template with `%(variable)` placeholders
- `method` - HTTP method (default: GET)
- `headers` - HTTP headers as key-value pairs
- `query` - Query parameters as key-value pairs
- `ttl` - How long to cache responses (duration string like "30s", "5m", "1h")
- `expiry` - How long to keep refreshing in background

#### AWS Secrets Manager Source

Fetches secrets from AWS Secrets Manager:

```toml
[source.database_secret]
type = "awssm"
key = "/app/%(instance)/database"
ttl = "5m"
expiry = "30m"
```

- `key` - Secret key template with `%(variable)` placeholders
- `ttl` - Cache TTL for secrets
- `expiry` - Background refresh duration

#### Merge Source

Combines multiple sources into a single JSON response:

```toml
[source.combined]
type = "merge"
format = "json"
template = [
  { key = ["config"], source = "app_config", args = { inherit = {} } },
  { key = ["secrets"], source = "app_secret", args = { replace = { instance = "{instance}" } } },
]
ttl = "60s"
expiry = "300s"
```

- `format` - Output format ("json")
- `template` - Array of merge rules
  - `key` - JSON path array where to place the result
  - `source` - Name of source to fetch from
  - `args` - How to handle variables:
    - `{ inherit = {} }` - Pass through all variables
    - `{ replace = { var = "value" } }` - Override specific variables
- `ttl` - Cache TTL for merged response
- `expiry` - Background refresh duration

### Duration Format

Duration strings support these formats:
- `"30s"` - 30 seconds
- `"5m"` - 5 minutes
- `"2h"` - 2 hours
- `"1d"` - 1 day

### Complete Example

```toml
[server]
target = { host = "localhost", port = 11213 }

[routes.app]
routes = [
  { match = "^(?<instance>[^/]+)/config$", to = "config" },
  { match = "^(?<instance>[^/]+)/secret$", to = "secret" },
  { match = "^(?<instance>[^/]+)/all$", to = "combined" },
]

[source.config]
type = "http"
url = "https://config-service/%(instance)/config"
ttl = "60s"
expiry = "300s"

[source.secret]
type = "awssm"
key = "/app/%(instance)/secret"
ttl = "300s"
expiry = "1800s"

[source.combined]
type = "merge"
format = "json"
template = [
  { key = ["config"], source = "config", args = { inherit = {} } },
  { key = ["secret"], source = "secret", args = { inherit = {} } },
]
ttl = "60s"
expiry = "300s"
```
