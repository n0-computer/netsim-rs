#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "setup.sh: this only works on Linux."
  exit 1
fi

if ! command -v setcap >/dev/null 2>&1; then
  echo "setup.sh: setcap not found. Install libcap2-bin (Debian/Ubuntu) or libcap (Fedora)."
  exit 1
fi

need_sudo=0
if [[ "$EUID" -ne 0 ]]; then
  need_sudo=1
  if ! command -v sudo >/dev/null 2>&1; then
    echo "setup.sh: sudo not found; run as root."
    exit 1
  fi
fi

caps="cap_net_admin,cap_sys_admin,cap_net_raw+ep"
maybe_sudo=()
if [[ "$need_sudo" -eq 1 ]]; then
  maybe_sudo=(sudo)
fi

echo "Granting capabilities to system tools (ip/tc/nft) if present..."
for bin in /usr/sbin/ip /sbin/ip /usr/bin/ip /usr/sbin/tc /sbin/tc /usr/bin/tc /usr/sbin/nft /sbin/nft /usr/bin/nft; do
  if [[ -x "$bin" ]]; then
    "${maybe_sudo[@]}" setcap "$caps" "$bin" || true
  fi
done

echo "Building tests (no run) to locate the test binary..."
cargo test --no-run

echo "Granting capabilities to netsim test binaries in target/debug/deps..."
shopt -s nullglob
for bin in target/debug/deps/netsim-*; do
  if [[ -x "$bin" && ! "$bin" =~ \.d$ ]]; then
    "${maybe_sudo[@]}" setcap "$caps" "$bin"
  fi
done

echo
echo "Setup complete."
echo "You can now run tests without sudo as long as you don't rebuild."
echo "If you rebuild, rerun ./setup.sh to reapply capabilities."
