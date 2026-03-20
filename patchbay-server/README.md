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
  --bind 0.0.0.0:8080 \
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
{"ok": true, "project": "myproject", "run": "myproject-20260320_120000-uuid", "invocation": "myproject-20260320_120000-uuid"}
```

The `invocation` value is used for deep linking: `https://your-server/#/inv/{invocation}`

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
| `--bind <addr>` | Listen address (default: `0.0.0.0:8080`, ignored with `--acme-domain`) |

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

Add this to your workflow after the test step:

```yaml
    - name: Push patchbay results
      if: always()
      env:
        PATCHBAY_URL: ${{ secrets.PATCHBAY_URL }}
        PATCHBAY_API_KEY: ${{ secrets.PATCHBAY_API_KEY }}
      run: |
        set -euo pipefail

        PROJECT="${{ github.event.repository.name }}"
        TESTDIR="$(cargo metadata --format-version=1 --no-deps | jq -r .target_directory)/testdir-current"

        if [ ! -d "$TESTDIR" ]; then
          echo "No testdir output found, skipping push"
          exit 0
        fi

        cat > "$TESTDIR/run.json" <<MANIFEST
        {
          "project": "$PROJECT",
          "branch": "${{ github.head_ref || github.ref_name }}",
          "commit": "${{ github.sha }}",
          "pr": ${{ github.event.pull_request.number || 'null' }},
          "pr_url": "${{ github.event.pull_request.html_url || '' }}",
          "title": "${{ github.event.pull_request.title || github.event.head_commit.message || '' }}",
          "created_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        }
        MANIFEST

        RESPONSE=$(tar -czf - -C "$TESTDIR" . | \
          curl -s -w "\n%{http_code}" \
            -X POST \
            -H "Authorization: Bearer $PATCHBAY_API_KEY" \
            -H "Content-Type: application/gzip" \
            --data-binary @- \
            "$PATCHBAY_URL/api/push/$PROJECT")

        HTTP_CODE=$(echo "$RESPONSE" | tail -1)
        BODY=$(echo "$RESPONSE" | head -n -1)

        if [ "$HTTP_CODE" != "200" ]; then
          echo "Push failed ($HTTP_CODE): $BODY"
          exit 1
        fi

        INVOCATION=$(echo "$BODY" | jq -r .invocation)
        VIEW_URL="$PATCHBAY_URL/#/inv/$INVOCATION"
        echo "PATCHBAY_VIEW_URL=$VIEW_URL" >> "$GITHUB_ENV"
        echo "Results uploaded: $VIEW_URL"

    - name: Comment on PR
      if: always() && github.event.pull_request && env.PATCHBAY_VIEW_URL
      uses: actions/github-script@v7
      with:
        script: |
          const marker = '<!-- patchbay-results -->';
          const body = `${marker}\n**patchbay results:** ${process.env.PATCHBAY_VIEW_URL}`;
          const { data: comments } = await github.rest.issues.listComments({
            owner: context.repo.owner,
            repo: context.repo.repo,
            issue_number: context.issue.number,
          });
          const existing = comments.find(c => c.body.includes(marker));
          if (existing) {
            await github.rest.issues.updateComment({
              owner: context.repo.owner,
              repo: context.repo.repo,
              comment_id: existing.id,
              body,
            });
          } else {
            await github.rest.issues.createComment({
              owner: context.repo.owner,
              repo: context.repo.repo,
              issue_number: context.issue.number,
              body,
            });
          }
```

The PR comment is auto-updated on each push so you always see the latest run.
