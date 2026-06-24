/**
 * Generate minimal placeholder PNG icons for the Tauri bundle so
 * `tauri::generate_context!()` succeeds during `cargo check` / `cargo build`.
 * Replace with real artwork before release.
 */
import { writeFileSync, mkdirSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import zlib from 'node:zlib'

const __dirname = dirname(fileURLToPath(import.meta.url))
const iconsDir = resolve(__dirname, '..', 'apps', 'core', 'src-tauri', 'icons')
mkdirSync(iconsDir, { recursive: true })

function makePng(size) {
  // Solid color RGBA (30,30,46,255) PNG.
  const width = size
  const height = size
  const raw = Buffer.alloc(height * (1 + width * 4))
  for (let y = 0; y < height; y++) {
    raw[y * (1 + width * 4)] = 0 // filter byte
    for (let x = 0; x < width; x++) {
      const off = y * (1 + width * 4) + 1 + x * 4
      raw[off] = 30
      raw[off + 1] = 30
      raw[off + 2] = 46
      raw[off + 3] = 255
    }
  }
  const compressed = zlib.deflateSync(raw)

  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])
  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(width, 0)
  ihdr.writeUInt32BE(height, 4)
  ihdr[8] = 8 // bit depth
  ihdr[9] = 6 // color type RGBA
  ihdr[10] = 0
  ihdr[11] = 0
  ihdr[12] = 0

  function chunk(type, data) {
    const len = Buffer.alloc(4)
    len.writeUInt32BE(data.length, 0)
    const typeBuf = Buffer.from(type, 'ascii')
    const crcInput = Buffer.concat([typeBuf, data])
    const crc = Buffer.alloc(4)
    crc.writeUInt32BE(crc32(crcInput), 0)
    return Buffer.concat([len, typeBuf, data, crc])
  }

  function crc32(buf) {
    let c = ~0
    for (let i = 0; i < buf.length; i++) {
      c ^= buf[i]
      for (let k = 0; k < 8; k++) {
        c = c & 1 ? (c >>> 1) ^ 0xedb88320 : c >>> 1
      }
    }
    return ~c >>> 0
  }

  const iend = Buffer.alloc(0)
  return Buffer.concat([
    sig,
    chunk('IHDR', ihdr),
    chunk('IDAT', compressed),
    chunk('IEND', iend),
  ])
}

for (const size of [32, 128, 256]) {
  const name = size === 256 ? '128x128@2x.png' : `${size}x${size}.png`
  writeFileSync(resolve(iconsDir, name), makePng(size))
}
writeFileSync(resolve(iconsDir, 'icon.png'), makePng(512))

// Minimal .ico wrapping a 32x32 PNG (ICO format: header + dir entry + PNG data).
function makeIco(png) {
  const header = Buffer.alloc(6)
  header.writeUInt16LE(0, 0)
  header.writeUInt16LE(1, 2)
  header.writeUInt16LE(1, 4)
  const dir = Buffer.alloc(16)
  dir[0] = 32 // width
  dir[1] = 32 // height
  dir[2] = 0
  dir[3] = 0
  dir.writeUInt16LE(1, 4)
  dir.writeUInt16LE(32, 6)
  dir.writeUInt32LE(png.length, 8)
  dir.writeUInt32LE(6 + 16, 12)
  return Buffer.concat([header, dir, png])
}
writeFileSync(resolve(iconsDir, 'icon.ico'), makeIco(makePng(32)))

// .icns: minimal header + PNG-based icon. Not strictly valid but unblocks build.
const icnsPng = makePng(128)
const icnsMagic = Buffer.from('icns', 'ascii')
const icnsType = Buffer.from('ic07', 'ascii') // 128x128 png
const icnsLen = Buffer.alloc(4)
icnsLen.writeUInt32BE(icnsType.length + icnsPng.length + 8, 0)
const icnsTotal = Buffer.alloc(4)
icnsTotal.writeUInt32BE(8 + icnsType.length + icnsPng.length + 8, 0)
writeFileSync(
  resolve(iconsDir, 'icon.icns'),
  Buffer.concat([icnsMagic, icnsTotal, icnsType, icnsLen, icnsPng]),
)

console.log('icons generated at', iconsDir)
