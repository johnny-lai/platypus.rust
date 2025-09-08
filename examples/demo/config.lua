local warm_port = os.getenv("WARM_PORT")
local cold_port = os.getenv("COLD_PORT")

pools {
    warm_pool = {
        backends = {
            "127.0.0.1:" .. warm_port,
        }
    },
    cold_pool = {
        backends = {
            "127.0.0.1:" .. cold_port,
        }
    }
}

routes {
    default = route_failover {
        children = { "warm_pool", "cold_pool" },
        miss = true, -- failover on miss
    }
}
