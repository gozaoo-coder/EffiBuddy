/**
 * Install a staged plugin into the Core packages data dir so it shows up
 * in the package list on next launch.
 *
 * Usage: node scripts/install-plugin.mjs <package-dir>
 */
import { existsSync, mkdirSync, readFileSync, cpSync } from 'node:fs'
import { resolve, dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { homedir } from 'node:os'

const __dirname = dirname(fileURLToPath(import.meta.url))
const root = resolve(__dirname, '..')

const pkgDir = process.argv[2]
if (!pkgDir) {
  console.error('Usage: install-plugin.mjs <package-dir>')
  process.exit(1)
}

const absPkg = resolve(root, pkgDir)
const manifestPath = join(absPkg, 'manifest.json')
if (!existsSync(manifestPath)) {
  console.error(`manifest.json not found in ${absPkg}`)
  process.exit(1)
}

const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
const dataDir = process.env.LOCALAPPDATA
  ? join(process.env.LOCALAPPDATA, 'desktop-suite', 'packages')
  : join(homedir(), '.local', 'share', 'desktop-suite', 'packages')

const dest = join(dataDir, manifest.id)
mkdirSync(dest, { recursive: true })

cpSync(join(absPkg, 'manifest.json'), join(dest, 'manifest.json'))
if (existsSync(join(absPkg, 'backend'))) {
  cpSync(join(absPkg, 'backend'), join(dest, 'backend'), { recursive: true })
}
if (existsSync(join(absPkg, 'frontend'))) {
  cpSync(join(absPkg, 'frontend'), join(dest, 'frontend'), { recursive: true })
}
if (existsSync(join(absPkg, 'assets'))) {
  cpSync(join(absPkg, 'assets'), join(dest, 'assets'), { recursive: true })
}

console.log(`[install-plugin] ${manifest.id} -> ${dest}`)
