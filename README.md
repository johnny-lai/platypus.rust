# platypus

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

## Sources

```
[get.<get_name>]
match = quickstart/(.*)
source = <source_name1>

[source.<source_name1>]
type = awssm
key = some/prefix/{1}

[source.<source_name2>]
type = http
url = http://localhost:123/stuff/{1}
action = GET

[target]
host = localhost
port = 11213
```
