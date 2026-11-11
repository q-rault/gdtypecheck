set dotenv-load
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

test:
    cargo test --quiet

build:
    cargo build --release --quiet
