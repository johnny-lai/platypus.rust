local warm_port = os.getenv("WARM_PORT")
local cold_port = os.getenv("COLD_PORT")

pools {
    quickstart_pool = {
        backends = {
            "127.0.0.1:" .. warm_port,
        }
    },
    fallback = {
        backends = {
            "127.0.0.1:" .. cold_port,
        }
    }
}

routes {
    map = {
        quickstart = route_direct {
            child = "quickstart_pool",
        },
    },
    default = route_direct { child = "fallback" }
}
