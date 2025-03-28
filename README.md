# NTP Time to REST Api middleware server
## Intro
This server connect to designeated NTP server and avail time through REST HTTP Requests

## Building
using default Rust cargo buid
```
cargo build --release
```

## Env file
- LOG_LEVEL: Log lever for the server (default :INFO, Values: Off, Error,Warn,Info,Debug,Trace)
- IP: Listining IP Adress (default: 0.0.0.0 )
- PORT: Listining IP Port (default: 3000)
- NTP_SERVER : Backend NTP backend server (default: time.google.com:123)
- CACHE_TIMEOUT: NTP call cache in secs (default 5mins:300)

## Endpoints
- /health: Service health endpoint
- /now : Return the time now in UTC
- /now/{continent}/{region} : Return the localized time (i.e. : /now/Europe/London)


