import { test, expect } from '@playwright/test'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { type ChildProcess, spawn } from 'node:child_process'
import { PATCHBAY_BIN, REPO_ROOT, waitForHttp } from './helpers'

const PORT = 7434
const UI_URL = `http://127.0.0.1:${PORT}`

const MINIMAL_EVENT =
  '{"opid":1,"timestamp":"2026-03-25T00:00:00Z","kind":"lab_created","lab_prefix":"lab-p1","label":"test"}\n'

const MOCK_METRICS = [
  '{"t":1,"m":{"packet_count":5.0}}',
  '{"t":2,"m":{"packet_count":5.0}}',
  '{"t":3,"m":{"packet_count":5.0}}',
].join('\n') + '\n'

const MOCK_MANIFEST = {
  left_ref: 'v1',
  right_ref: 'v2',
  timestamp: '20260325_120000',
  left_results: [
    { name: 'counter::udp_counter', status: 'pass', duration_ms: 100 },
    { name: 'counter::udp_threshold', status: 'pass', duration_ms: 50 },
  ],
  right_results: [
    { name: 'counter::udp_counter', status: 'pass', duration_ms: 110 },
    { name: 'counter::udp_threshold', status: 'fail', duration_ms: 40 },
  ],
  summary: {
    left_pass: 2,
    left_fail: 0,
    left_total: 2,
    right_pass: 1,
    right_fail: 1,
    right_total: 2,
    fixes: 0,
    regressions: 1,
    left_time_ms: 150,
    right_time_ms: 150,
    score: -5,
  },
}

test('compare view renders summary and regression', async ({ page }) => {
  test.setTimeout(60_000)
  const workDir = mkdtempSync(join(tmpdir(), 'patchbay-compare-e2e-'))
  let proc: ChildProcess | null = null

  try {
    // Write mock data
    const batchDir = join(workDir, 'compare-mock')
    mkdirSync(join(batchDir, 'left-v1'), { recursive: true })
    mkdirSync(join(batchDir, 'right-v2'), { recursive: true })
    writeFileSync(join(batchDir, 'summary.json'), JSON.stringify(MOCK_MANIFEST))
    writeFileSync(join(batchDir, 'left-v1', 'events.jsonl'), MINIMAL_EVENT)
    writeFileSync(join(batchDir, 'right-v2', 'events.jsonl'), MINIMAL_EVENT)
    writeFileSync(
      join(batchDir, 'right-v2', 'device.sender.metrics.jsonl'),
      MOCK_METRICS,
    )

    // Start server
    proc = spawn(
      PATCHBAY_BIN,
      ['serve', workDir, '--bind', `127.0.0.1:${PORT}`],
      { cwd: REPO_ROOT, stdio: 'pipe' },
    )
    await waitForHttp(UI_URL, 15_000)

    // Navigate to the app
    await page.goto(UI_URL)

    // Select the compare batch
    await page.waitForTimeout(1000)
    // Look for compare-mock in the page (it should be in the run list)
    const batchLink = page.getByText('compare-mock')
    if (await batchLink.isVisible()) {
      await batchLink.click()
    } else {
      // Try selector
      const selector = page.locator('select')
      if (await selector.isVisible()) {
        await selector.selectOption({ label: 'compare-mock' })
      }
    }

    // Verify CompareView renders
    await expect(page.getByText('v1')).toBeVisible({ timeout: 10_000 })
    await expect(page.getByText('v2')).toBeVisible()

    // Summary
    await expect(page.getByText('Regressions')).toBeVisible()

    // Per-test table
    await expect(page.getByText('udp_counter')).toBeVisible()
    await expect(page.getByText('udp_threshold')).toBeVisible()
    await expect(page.getByText('REGRESS')).toBeVisible()

    // Score
    await expect(page.getByText('-5')).toBeVisible()
  } finally {
    if (proc && !proc.killed) proc.kill('SIGTERM')
    rmSync(workDir, { recursive: true, force: true })
  }
})
