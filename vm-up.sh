#!/usr/bin/env bash
set -euo pipefail

NAME="${NAME:-netsim-vm}"
WORKSPACE="$(pwd -P)"
EXTRA="${1:-}"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

cat >"$TMP" <<EOF
images:
  - location: "https://cloud.debian.org/images/cloud/trixie/latest/debian-13-genericcloud-amd64.qcow2"
    arch: "x86_64"

mounts:
  - location: "$WORKSPACE"
    mountPoint: /app
    writable: true
EOF

if [[ -n "$EXTRA" ]]; then
cat >>"$TMP" <<EOF
  - location: "$EXTRA"
    mountPoint: /target
    writable: true
EOF
fi

cat >>"$TMP" <<'EOF'

containerd:
  system: false
  user: false

provision:
  - mode: system
    script: |
      #!/bin/bash
      set -eux -o pipefail
      sudo modprobe sch_netem
      echo sch_netem | sudo tee -a /etc/modules
      export DEBIAN_FRONTEND=noninteractive
      apt update
      apt install -y bridge-utils iproute2 iputils-ping iptables nftables net-tools curl iperf3
EOF

if limactl list "$NAME" 2>/dev/null | grep -q "$NAME"; then
  limactl stop "$NAME" >/dev/null 2>&1 || true
  limactl delete -f "$NAME"
fi

limactl start --tty=false --name "$NAME" "$TMP"
