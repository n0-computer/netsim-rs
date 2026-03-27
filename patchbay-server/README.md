# patchbay-serve

Standalone server for hosting patchbay run results. CI pipelines push
test output to it; the devtools UI lets you browse them.

## Install

```bash
cargo install --git https://github.com/n0-computer/patchbay patchbay-server --bin patchbay-serve
```

## Quick start

```bash
patchbay-serve \
  --accept-push \
  --api-key "$(openssl rand -hex 32)" \
  --http-bind 0.0.0.0:8080 \
  --retention 10GB
```

With automatic TLS via Let's Encrypt:

```bash
patchbay-serve \
  --accept-push \
  --api-key "$(openssl rand -hex 32)" \
  --acme-domain patchbay.example.com \
  --acme-email you@example.com \
  --retention 10GB
```

This will:

- Serve the devtools UI at `/` with a runs index
- Accept pushed runs at `POST /api/push/{project}`
- Auto-provision TLS via Let's Encrypt (when `--acme-domain` is set)
- Store data in `~/.local/share/patchbay-serve/` (runs + ACME certs)
- Delete oldest runs when total size exceeds the retention limit

## Push API

```
POST /api/push/{project}
Authorization: Bearer <api-key>
Content-Type: application/gzip
Body: tar.gz of the run directory
```

Returns:

```json
{"ok": true, "project": "myproject", "run": "myproject-20260320_120000-uuid", "group": "myproject-20260320_120000-uuid"}
```

The `group` value is used for deep linking: `https://your-server/batch/{group}`

## Flags

| Flag | Description |
|------|-------------|
| `--run-dir <path>` | Override run storage location |
| `--data-dir <path>` | Override data directory (default: `~/.local/share/patchbay-serve`) |
| `--accept-push` | Enable the push API |
| `--api-key <key>` | Required with `--accept-push`; also reads `PATCHBAY_API_KEY` env var |
| `--acme-domain <d>` | Enable automatic TLS for this domain |
| `--acme-email <e>` | Contact email for Let's Encrypt (required with `--acme-domain`) |
| `--retention <size>` | Max total run storage (e.g. `500MB`, `10GB`) |
| `--http-bind <addr>` | HTTP listen address (default: `0.0.0.0:8080`; redirect when ACME is active) |
| `--https-bind <addr>` | HTTPS listen address (default: `0.0.0.0:4443`; only used with `--acme-domain`) |

## Deploy with systemd

A unit file is included at [`patchbay-serve.service`](patchbay-serve.service).

### 1. Create a service user and data directory

```bash
sudo useradd -r -s /usr/sbin/nologin -d /var/lib/patchbay-serve patchbay
sudo mkdir -p /var/lib/patchbay-serve
sudo chown patchbay:patchbay /var/lib/patchbay-serve
```

### 2. Install the binary

```bash
cargo install --git https://github.com/n0-computer/patchbay patchbay-server --bin patchbay-serve
sudo cp ~/.cargo/bin/patchbay-serve /usr/local/bin/
```

### 3. Install and configure the unit file

```bash
sudo cp patchbay-serve.service /etc/systemd/system/
```

Edit the service to set your domain, email, and API key:

```bash
sudo systemctl edit patchbay-serve
```

```ini
[Service]
ExecStart=
ExecStart=/usr/local/bin/patchbay-serve \
    --accept-push \
    --data-dir /var/lib/patchbay-serve \
    --http-bind 0.0.0.0:80 \
    --https-bind 0.0.0.0:443 \
    --acme-domain patchbay.yourcompany.com \
    --acme-email ops@yourcompany.com \
    --retention 10GB
Environment=PATCHBAY_API_KEY=your-secret-key-here
```

### 4. Start

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now patchbay-serve
```

### 5. Verify

```bash
sudo systemctl status patchbay-serve
journalctl -u patchbay-serve -f
```

The service runs with hardened settings (`ProtectSystem=strict`,
`ProtectHome=true`, `NoNewPrivileges=true`) and only has write access
to `/var/lib/patchbay-serve`.

## GitHub Actions

Set two repository secrets: `PATCHBAY_URL` (e.g. `https://patchbay.yourcompany.com`)
and `PATCHBAY_API_KEY`.

A complete workflow template is at [`github-workflow-template.yml`](github-workflow-template.yml).
Copy it into your repo's `.github/workflows/` and adjust the test command. It handles
pushing results, writing `run.json` with CI context, and posting/updating a PR comment
with the results link, commit hash, and timestamp.
