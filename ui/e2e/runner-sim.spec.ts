import { expect, test } from '@playwright/test'
import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { REPO_ROOT, PATCHBAY_BIN, waitForHttp } from './helpers'

const THIS_DIR = path.dirname(fileURLToPath(import.meta.url))
const PING_TOML = path.join(THIS_DIR, 'fixtures', 'ping-e2e.toml')
const IPERF_TOML = path.join(THIS_DIR, 'fixtures', 'iperf-e2e.toml')
const UI_BIND = '127.0.0.1:7432'
const UI_URL = `http://${UI_BIND}/`

test('runner sim produces viewable UI output', async ({ page }) => {
  test.setTimeout(4 * 60 * 1000)
  const workDir = mkdtempSync(`${tmpdir()}/patchbay-runner-e2e-`)
  let serveProc: ChildProcess | null = null
  try {
    // Step 1: Run the sim.
    execFileSync(
      PATCHBAY_BIN,
      ['run', '--work-dir', workDir, PING_TOML],
      {
        cwd: REPO_ROOT,
        stdio: 'inherit',
        env: process.env,
        timeout: 2 * 60 * 1000,
      },
    )

    // Step 2: Start the devtools server.
    serveProc = spawn(
      PATCHBAY_BIN,
      ['serve', workDir, '--bind', UI_BIND],
      { cwd: REPO_ROOT, stdio: 'inherit' },
    )
    await waitForHttp(UI_URL, 15_000)

    // Step 3: Verify the runs index shows the run.
    await page.goto(UI_URL)
    await expect(page.getByRole('heading', { name: 'Runs' })).toBeVisible({ timeout: 15_000 })

    // Click on the run entry to navigate to the run detail.
    const runLink = page.locator('a[href*="/run/"]').first()
    await expect(runLink).toBeVisible({ timeout: 10_000 })
    await runLink.click()

    // Topology tab should show the router and devices.
    await expect(page.getByText('dc')).toBeVisible({ timeout: 10_000 })
    await expect(page.getByText('sender')).toBeVisible()
    await expect(page.getByText('receiver')).toBeVisible()

    // Logs tab: events.jsonl should show lab events.
    await page.getByRole('button', { name: 'logs' }).click()
    await expect(page.getByText('events.jsonl').first()).toBeVisible({ timeout: 5_000 })
    await page.getByText('events.jsonl').first().click()
    const eventsTable = page.locator('table tbody tr')
    await expect(eventsTable.first()).toBeVisible({ timeout: 5_000 })
    await expect(page.getByText('router_added').first()).toBeVisible()
    await expect(page.getByText('device_added').first()).toBeVisible()

    // Perf tab: should show latency column from ping results.
    await page.getByRole('button', { name: 'perf' }).click()
    await expect(page.getByText('ping-check')).toBeVisible({ timeout: 5_000 })
    await expect(page.getByText('Latency (ms)')).toBeVisible()
  } finally {
    if (serveProc && !serveProc.killed) {
      serveProc.kill('SIGTERM')
    }
    rmSync(workDir, { recursive: true, force: true })
  }
})

test('multi-sim batch shows grouped selector and combined results', async ({ page }) => {
  test.setTimeout(4 * 60 * 1000)
  const workDir = mkdtempSync(`${tmpdir()}/patchbay-runner-e2e-multi-`)
  let serveProc: ChildProcess | null = null
  try {
    // Run both sims in a single batch.
    execFileSync(
      PATCHBAY_BIN,
      ['run', '--work-dir', workDir, PING_TOML, IPERF_TOML],
      {
        cwd: REPO_ROOT,
        stdio: 'inherit',
        env: process.env,
        timeout: 2 * 60 * 1000,
      },
    )

    // Start devtools server.
    serveProc = spawn(
      PATCHBAY_BIN,
      ['serve', workDir, '--bind', UI_BIND],
      { cwd: REPO_ROOT, stdio: 'inherit' },
    )
    await waitForHttp(UI_URL, 15_000)

    await page.goto(UI_URL)
    await expect(page.getByRole('heading', { name: 'Runs' })).toBeVisible({ timeout: 15_000 })
    // Both sims should appear as run entries in the index.
    await expect(page.getByText('ping-e2e').first()).toBeVisible({ timeout: 10_000 })
    await expect(page.getByText('iperf-e2e').first()).toBeVisible()
  } finally {
    if (serveProc && !serveProc.killed) {
      serveProc.kill('SIGTERM')
    }
    rmSync(workDir, { recursive: true, force: true })
  }
})

test('iperf sim shows perf results', async ({ page }) => {
  test.setTimeout(4 * 60 * 1000)
  const workDir = mkdtempSync(`${tmpdir()}/patchbay-runner-e2e-iperf-`)
  let serveProc: ChildProcess | null = null
  try {
    // Run the iperf sim.
    execFileSync(
      PATCHBAY_BIN,
      ['run', '--work-dir', workDir, IPERF_TOML],
      {
        cwd: REPO_ROOT,
        stdio: 'inherit',
        env: process.env,
        timeout: 2 * 60 * 1000,
      },
    )

    // Start devtools server.
    serveProc = spawn(
      PATCHBAY_BIN,
      ['serve', workDir, '--bind', UI_BIND],
      { cwd: REPO_ROOT, stdio: 'inherit' },
    )
    await waitForHttp(UI_URL, 15_000)

    await page.goto(UI_URL)
    await expect(page.getByRole('heading', { name: 'Runs' })).toBeVisible({ timeout: 15_000 })
    // Click through to the run detail.
    const runLink = page.locator('a[href*="/run/"]').first()
    await expect(runLink).toBeVisible({ timeout: 10_000 })
    await runLink.click()

    // Navigate to perf tab.
    await page.getByRole('button', { name: 'perf' }).click()
    await expect(page.getByText('iperf-client')).toBeVisible({ timeout: 5_000 })
    await expect(page.getByText('Down MB/s')).toBeVisible()
  } finally {
    if (serveProc && !serveProc.killed) {
      serveProc.kill('SIGTERM')
    }
    rmSync(workDir, { recursive: true, force: true })
  }
})
