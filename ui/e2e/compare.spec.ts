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

const MOCK_LEFT_MANIFEST = {
  kind: 'test',
  project: 'test-project',
  commit: 'aaa111',
  branch: 'main',
  dirty: false,
  outcome: 'pass',
  pass: 2,
  fail: 0,
  total: 2,
  tests: [
    { name: 'counter::udp_counter', status: 'pass' },
    { name: 'counter::udp_threshold', status: 'pass' },
  ],
}

const MOCK_RIGHT_MANIFEST = {
  kind: 'test',
  project: 'test-project',
  commit: 'bbb222',
  branch: 'feature',
  dirty: false,
  outcome: 'fail',
  pass: 1,
  fail: 1,
  total: 2,
  tests: [
    { name: 'counter::udp_counter', status: 'pass' },
    { name: 'counter::udp_threshold', status: 'fail' },
  ],
}

test('compare view renders summary and regression', async ({ page }) => {
  test.setTimeout(60_000)
  const workDir = mkdtempSync(join(tmpdir(), 'patchbay-compare-e2e-'))
  let proc: ChildProcess | null = null

  try {
    // Write mock data: two separate run directories, each with run.json
    const leftDir = join(workDir, 'run-left')
    const rightDir = join(workDir, 'run-right')
    mkdirSync(leftDir, { recursive: true })
    mkdirSync(rightDir, { recursive: true })

    writeFileSync(join(leftDir, 'run.json'), JSON.stringify(MOCK_LEFT_MANIFEST))
    writeFileSync(join(leftDir, 'events.jsonl'), MINIMAL_EVENT)

    writeFileSync(join(rightDir, 'run.json'), JSON.stringify(MOCK_RIGHT_MANIFEST))
    writeFileSync(join(rightDir, 'events.jsonl'), MINIMAL_EVENT)

    // Start server
    proc = spawn(
      PATCHBAY_BIN,
      ['serve', workDir, '--bind', `127.0.0.1:${PORT}`],
      { cwd: REPO_ROOT, stdio: 'pipe' },
    )
    await waitForHttp(UI_URL, 15_000)

    // Navigate directly to the compare view with two run names
    await page.goto(`${UI_URL}/compare/run-left/run-right`)

    // Verify CompareView renders with ref labels (appears in heading + summary + table)
    await expect(page.getByText('main@aaa111').first()).toBeVisible({ timeout: 10_000 })
    await expect(page.getByText('feature@bbb222').first()).toBeVisible()

    // Summary
    await expect(page.getByText('Regressions')).toBeVisible()

    // Per-test table
    await expect(page.getByText('udp_counter')).toBeVisible()
    await expect(page.getByText('udp_threshold')).toBeVisible()
    await expect(page.getByText('REGRESS').first()).toBeVisible()

    // Score: 0 fixes, 1 regression => score = -5
    await expect(page.getByText('-5')).toBeVisible()
  } finally {
    if (proc && !proc.killed) proc.kill('SIGTERM')
    rmSync(workDir, { recursive: true, force: true })
  }
})
