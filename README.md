# procyon

Periodically fetches answers in the background into memcached
* On `get`, procyon will start fetching in the background and updating the "target" memcache, with a TTL
* After a certain time the TTL on the target will expire, and then new `get` requests will come back to procyon.
  This is an indication that the key is still needed, and procyon will extend the background fetch time.
  
```
Client --> McRouter/memcached-proxy ---> target memcache
                        |                           ^
                        \                           |
                         \- on miss ---> procyon --/
                                          |  ^
                                          |  |
                                          v  |
                                      actual service
```
