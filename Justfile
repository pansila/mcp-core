server:
    cargo run --bin server

sse-server:
    cargo run --bin server -- -t sse

sse-client:
    cargo run --bin client -- -t sse list-tools