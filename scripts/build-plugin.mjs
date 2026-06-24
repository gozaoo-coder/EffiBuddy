/**
 * Build a plugin package: compiles the Rust backend (cdylib) and the
 * Vue frontend, then copies them into a staging dir matching the
 * manifest layout so Core can install it.
 *
 * Usage: node scripts/build-plugin.mjs <package-dir>
 */
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync, cpSync } from 'node:fs'
import { resolve, dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { execSync } from 'node:child_process'

const __dirname = dirname(fileURLToPath(import.meta.url))
const root = resolve(__dirname, '..')

const pkgDir = process.argv[2]
if (!pkgDir) {
  console.error('Usage: build-plugin.mjs <package-dir>')
  process.exit(1)
}

const absPkg = resolve(root, pkgDir)
const manifestPath = join(absPkg, 'manifest.json')
if (!existsSync(manifestPath)) {
  console.error(`manifest.json not found in ${absPkg}`)
  process.exit(1)
}

const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
const staging = join(absPkg, '.staging')
rmSync(staging, { recursive: true, force: true })
mkdirSync(staging, { recursive: true })

// 1. Build backend if present.
if (existsSync(join(absPkg, 'backend'))) {
  console.log(`[build-plugin] compiling backend for ${manifest.id}`)
  execSync('cargo build --release', { cwd: join(absPkg, 'backend'), stdio: 'inherit' })
  const targetDir = join(absPkg, 'backend', 'target', 'release')
  const ext = process.platform === 'win32' ? '.dll' : process.platform === 'darwin' ? '.dylib' : '.so'
  const libName = manifest.entry?.backend?.replace(ext, '') ?? ''
  // Copy the produced cdylib next to manifest under the declared name.
  const candidates = [
    join(targetDir, libName + ext),
    join(targetDir, libName.replace(/-/g, '_') + ext),
  ]
  const found = candidates.find((p) => existsSync(p))
  if (found) {
    cpSync(found, join(staging, manifest.entry.backend))
  } else {
    console.warn(`[build-plugin] backend lib not found in ${targetDir}`)
  }
}

// 2. Build frontend if present.
if (existsSync(join(absPkg, 'frontend'))) {
  console.log(`[build-plugin] building frontend for ${manifest.id}`)
  execSync('pnpm install && pnpm vite build', {
    cwd: join(absPkg, 'frontend'),
    stdio: 'inherit',
  })
  const dist = join(absPkg, 'frontend', 'dist')
  if (existsSync(dist)) {
    cpSync(dist, join(staging, 'frontend'), { recursive: true })
  }
}

// 3. Copy manifest + assets.
cpSync(manifestPath, join(staging, 'manifest.json'))
if (existsSync(join(absPkg, 'assets'))) {
  cpSync(join(absPkg, 'assets'), join(staging, 'assets'), { recursive: true })
}

writeFileSync(join(staging, '.built'), new Date().toISOString())
console.log(`[build-plugin] staged at ${staging}`)
