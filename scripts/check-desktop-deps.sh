#!/usr/bin/env bash
# Fails if the desktop app's real dependency tree, for each shipped target,
# contains an HTTP client, TLS stack or socket library, or enables tokio's
# networking feature. Cargo.lock alone is not enough: it also lists
# mobile-only dependencies that never compile into the desktop app.
set -euo pipefail
forbidden='^(reqwest|hyper|hyper-util|h2|h3|quinn|rustls|native-tls|openssl|socket2|mio|ureq|curl|isahc|surf|tungstenite|attohttpc|hickory-resolver|trust-dns-resolver)$'
status=0
for target in x86_64-pc-windows-msvc aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu; do
  ok=1
  found=$(cargo tree -p scout-app -e normal --target "$target" --prefix none --locked | awk '{print $1}' | sort -u | grep -E "$forbidden" || true)
  if [ -n "$found" ]; then
    echo "::error::$target: networking crates in desktop build: $found"; status=1; ok=0
  fi
  if cargo tree -p scout-app -e features --target "$target" -i tokio --locked 2>/dev/null | grep -q 'tokio feature "net"'; then
    echo "::error::$target: tokio's network feature is enabled"; status=1; ok=0
  fi
  [ "$ok" = 1 ] && echo "$target: ok"
done
exit $status
