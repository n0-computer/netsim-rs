import { execFileSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const THIS_DIR = path.dirname(fileURLToPath(import.meta.url))
const UI_DIR = path.resolve(THIS_DIR, '..')
const REPO_ROOT = path.resolve(UI_DIR, '..')

export default function globalSetup() {
  // cargo build triggers npm build via patchbay-server's build.rs
  console.log('[setup] building cargo workspace (includes UI build)...')
  execFileSync('cargo', ['build', '-p', 'patchbay-cli', '-p', 'patchbay-server'], {
    cwd: REPO_ROOT,
    stdio: 'inherit',
    timeout: 5 * 60_000,
  })
}
