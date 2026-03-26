import { expect, test } from '@playwright/test'
import { execFileSync, execSync, spawn, type ChildProcess } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { REPO_ROOT, PATCHBAY_BIN, PATCHBAY_SERVE_BIN, waitForHttp } from './helpers'

const THIS_DIR = path.dirname(fileURLToPath(import.meta.url))
const PING_TOML = path.join(THIS_DIR, 'fixtures', 'ping-e2e.toml')
const SERVE_BIND = '127.0.0.1:7433'
const SERVE_URL = `http://${SERVE_BIND}`
const API_KEY = 'test-e2e-key-123'

test('push run results and view via deep link', async ({ page }) => {
  test.setTimeout(4 * 60 * 1000)
  const simWorkDir = mkdtempSync(`${tmpdir()}/patchbay-push-sim-`)
  const serveDataDir = mkdtempSync(`${tmpdir()}/patchbay-push-serve-`)
  let serveProc: ChildProcess | null = null

  try {
    // Step 1: Run a sim to create output.
    execFileSync(
      PATCHBAY_BIN,
      ['run', '--work-dir', simWorkDir, PING_TOML],
      {
        cwd: REPO_ROOT,
        stdio: 'inherit',
        env: process.env,
        timeout: 2 * 60 * 1000,
      },
    )

    // Resolve the latest run directory (follows the "latest" symlink).
    const latestDir = execSync(`readlink -f ${simWorkDir}/latest`, {
      encoding: 'utf-8',
    }).trim()

    // Write a run.json manifest into the output.
    writeFileSync(
      path.join(latestDir, 'run.json'),
      JSON.stringify({
        project: 'test-project',
        branch: 'feat/test',
        commit: 'abc1234',
        pr: 42,
        pr_url: 'https://github.com/example/repo/pull/42',
        title: 'E2E push test',
        created_at: new Date().toISOString(),
      }),
    )

    // Step 2: Start patchbay-serve with push enabled.
    serveProc = spawn(
      PATCHBAY_SERVE_BIN,
      [
        '--accept-push',
        '--api-key', API_KEY,
        '--data-dir', serveDataDir,
        '--http-bind', SERVE_BIND,
      ],
      { cwd: REPO_ROOT, stdio: 'inherit' },
    )
    await waitForHttp(`${SERVE_URL}/api/runs`, 15_000)

    // Step 3: Tar+gz the run output and push it.
    const tarGz = execSync(`tar -czf - -C "${latestDir}" .`)
    const pushRes = await fetch(`${SERVE_URL}/api/push/test-project`, {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${API_KEY}`,
        'Content-Type': 'application/gzip',
      },
      body: tarGz,
    })
    expect(pushRes.status).toBe(200)
    const pushBody = await pushRes.json() as { ok: boolean; group: string; batch: string; project: string }
    expect(pushBody.ok).toBe(true)
    expect(pushBody.project).toBe('test-project')
    expect(pushBody.group).toBeTruthy()

    // Step 4: Verify the run appears in the API (allow time for discovery).
    await new Promise(r => setTimeout(r, 3000))
    const runsRes = await fetch(`${SERVE_URL}/api/runs`)
    const runs = await runsRes.json() as Array<{ name: string; group: string | null }>
    expect(runs.length).toBeGreaterThan(0)
    // The pushed run should belong to a group matching the push dir.
    const run = runs.find(r => r.group === pushBody.group)
    expect(run).toBeTruthy()

    // Step 5: Open the runs index and verify pushed run appears with manifest data.
    await page.goto(SERVE_URL)
    await expect(page.getByRole('heading', { name: 'Runs' })).toBeVisible({ timeout: 15_000 })

    // The run should be visible. RunRow renders branch@commit from manifest.
    // For grouped runs, ManifestGroupHeader also shows project, PR link, title.
    const runEntry = page.locator('.run-entry, .pushed-run-entry').first()
    await expect(runEntry).toBeVisible({ timeout: 10_000 })
    // Branch@commit should appear (RunRow renders this from manifest)
    await expect(page.getByText('feat/test').first()).toBeVisible({ timeout: 5_000 })
    await expect(page.getByText('abc1234').first()).toBeVisible()

    // Step 6: Verify push auth — request without key should fail.
    const noAuthRes = await fetch(`${SERVE_URL}/api/push/test-project`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/gzip' },
      body: tarGz,
    })
    expect(noAuthRes.status).toBe(401)

    // Step 7: Verify /api/runs includes manifest data from run.json.
    const allRunsRes = await fetch(`${SERVE_URL}/api/runs`)
    const allRuns = await allRunsRes.json() as Array<{ name: string; manifest?: Record<string, unknown> | null }>
    const withManifest = allRuns.find((r) => r.manifest)
    expect(withManifest).toBeTruthy()
    expect(withManifest!.manifest!.branch).toBe('feat/test')
    expect(withManifest!.manifest!.pr).toBe(42)
  } finally {
    if (serveProc && !serveProc.killed) {
      serveProc.kill('SIGTERM')
    }
    rmSync(simWorkDir, { recursive: true, force: true })
    rmSync(serveDataDir, { recursive: true, force: true })
  }
})
