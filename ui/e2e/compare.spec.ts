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

test('checkbox selection on runs index navigates to compare view', async ({ page }) => {
  test.setTimeout(60_000)
  const workDir = mkdtempSync(join(tmpdir(), 'patchbay-compare-select-'))
  let proc: ChildProcess | null = null

  try {
    // Create two run directories with manifests
    const leftDir = join(workDir, 'run-left')
    const rightDir = join(workDir, 'run-right')
    mkdirSync(leftDir, { recursive: true })
    mkdirSync(rightDir, { recursive: true })

    writeFileSync(join(leftDir, 'run.json'), JSON.stringify(MOCK_LEFT_MANIFEST))
    writeFileSync(join(leftDir, 'events.jsonl'), MINIMAL_EVENT)
    writeFileSync(join(rightDir, 'run.json'), JSON.stringify(MOCK_RIGHT_MANIFEST))
    writeFileSync(join(rightDir, 'events.jsonl'), MINIMAL_EVENT)

    proc = spawn(
      PATCHBAY_BIN,
      ['serve', workDir, '--bind', `127.0.0.1:${PORT}`],
      { cwd: REPO_ROOT, stdio: 'pipe' },
    )
    await waitForHttp(UI_URL, 15_000)

    await page.goto(UI_URL)
    await expect(page.getByRole('heading', { name: 'Runs' })).toBeVisible({ timeout: 10_000 })

    // Both runs should appear
    const checkboxes = page.locator('.run-entry input[type="checkbox"]')
    await expect(checkboxes).toHaveCount(2, { timeout: 10_000 })

    // Compare button should NOT be visible with 0 selected
    await expect(page.locator('.compare-selected-btn')).not.toBeVisible()

    // Select first checkbox
    await checkboxes.first().check()
    // Compare button still not visible with only 1 selected
    await expect(page.locator('.compare-selected-btn')).not.toBeVisible()

    // Select second checkbox
    await checkboxes.nth(1).check()
    // Now the compare button should appear
    const compareBtn = page.locator('.compare-selected-btn')
    await expect(compareBtn).toBeVisible()
    await expect(compareBtn).toHaveText('Compare Selected (2)')

    // Click compare and verify navigation to compare view
    await compareBtn.click()
    await expect(page).toHaveURL(/\/compare\//)
    await expect(page.getByText('main@aaa111').first()).toBeVisible({ timeout: 10_000 })
    await expect(page.getByText('feature@bbb222').first()).toBeVisible()
  } finally {
    if (proc && !proc.killed) proc.kill('SIGTERM')
    rmSync(workDir, { recursive: true, force: true })
  }
})

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

    // Verify header shows ref labels and pass/fail summary
    await expect(page.getByText('main@aaa111').first()).toBeVisible({ timeout: 10_000 })
    await expect(page.getByText('feature@bbb222').first()).toBeVisible()
    // Concise header: "2/2 → 1/2 (1 regression)"
    await expect(page.getByText('2/2').first()).toBeVisible()
    await expect(page.getByText('1/2').first()).toBeVisible()
    await expect(page.getByText('regression').first()).toBeVisible()

    // Negative: no fixes in this scenario
    await expect(page.getByText('fix').first()).not.toBeVisible()

    // Score: 0 fixes, 1 regression => score = -5
    await expect(page.getByText('-5').first()).toBeVisible()

    // Per-test table: verify column content, not just presence
    const tableRows = page.locator('table tbody tr')
    await expect(tableRows).toHaveCount(2) // two tests total

    // udp_counter: pass on both sides, no delta
    const counterRow = tableRows.filter({ hasText: 'udp_counter' })
    await expect(counterRow.locator('td').nth(1)).toHaveText('PASS')  // left status
    await expect(counterRow.locator('td').nth(2)).toHaveText('PASS')  // right status
    await expect(counterRow.locator('td').nth(3)).toHaveText('')      // no delta

    // udp_threshold: pass -> fail = REGRESS
    const thresholdRow = tableRows.filter({ hasText: 'udp_threshold' })
    await expect(thresholdRow.locator('td').nth(1)).toHaveText('PASS')
    await expect(thresholdRow.locator('td').nth(2)).toHaveText('FAIL')
    await expect(thresholdRow.locator('td').nth(3)).toHaveText('REGRESS')
  } finally {
    if (proc && !proc.killed) proc.kill('SIGTERM')
    rmSync(workDir, { recursive: true, force: true })
  }
})

test('compare view shows fix when right side improves', async ({ page }) => {
  test.setTimeout(60_000)
  const workDir = mkdtempSync(join(tmpdir(), 'patchbay-compare-fix-'))
  let proc: ChildProcess | null = null

  try {
    // Reverse direction: left has a failure, right fixes it
    const leftDir = join(workDir, 'run-broken')
    const rightDir = join(workDir, 'run-fixed')
    mkdirSync(leftDir, { recursive: true })
    mkdirSync(rightDir, { recursive: true })

    writeFileSync(join(leftDir, 'run.json'), JSON.stringify(MOCK_RIGHT_MANIFEST)) // fail side
    writeFileSync(join(leftDir, 'events.jsonl'), MINIMAL_EVENT)
    writeFileSync(join(rightDir, 'run.json'), JSON.stringify(MOCK_LEFT_MANIFEST)) // pass side
    writeFileSync(join(rightDir, 'events.jsonl'), MINIMAL_EVENT)

    proc = spawn(
      PATCHBAY_BIN,
      ['serve', workDir, '--bind', `127.0.0.1:${PORT}`],
      { cwd: REPO_ROOT, stdio: 'pipe' },
    )
    await waitForHttp(UI_URL, 15_000)

    await page.goto(`${UI_URL}/compare/run-broken/run-fixed`)

    // Header should show fix info
    await expect(page.getByText('fix').first()).toBeVisible({ timeout: 10_000 })
    // Negative: no regressions
    await expect(page.getByText('regression')).not.toBeVisible()

    // Score: 1 fix * 3 = +3
    await expect(page.getByText('+3').first()).toBeVisible()

    // Delta column should show "fixed" not "REGRESS"
    const thresholdRow = page.locator('table tbody tr').filter({ hasText: 'udp_threshold' })
    await expect(thresholdRow.locator('td').nth(3)).toHaveText('fixed')
  } finally {
    if (proc && !proc.killed) proc.kill('SIGTERM')
    rmSync(workDir, { recursive: true, force: true })
  }
})
