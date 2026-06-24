/**
 * TypeScript types mirroring the Rust manifest schema.
 */
export interface ManifestEntry {
  backend?: string
  frontend?: string
}

export interface ManifestWidget {
  type: string
  name: string
  default_size?: [number, number]
}

export interface ManifestHooks {
  on_install?: string
  on_uninstall?: string
}

export interface Manifest {
  id: string
  name: string
  version: string
  core_version: string
  description?: string
  author?: string
  permissions: string[]
  entry: ManifestEntry
  widgets: ManifestWidget[]
  hooks: ManifestHooks
  signature?: string
}
