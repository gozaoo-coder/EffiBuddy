/**
 * Package management composable. Wraps package_* Tauri commands.
 */
import { useRustCommand } from './useRustCommand'

export interface WidgetInfo {
  type: string
  name: string
  default_size?: [number, number]
}

export interface PackageInfo {
  id: string
  name: string
  version: string
  description?: string
  author?: string
  permissions: string[]
  enabled: boolean
  has_backend: boolean
  has_frontend: boolean
  widgets: WidgetInfo[]
}

export function usePackage() {
  const invoke = useRustCommand()

  async function listPackages(): Promise<PackageInfo[]> {
    return invoke<PackageInfo[]>('list_packages')
  }

  async function installPackage(srcDir: string): Promise<PackageInfo> {
    return invoke<PackageInfo>('install_package', { srcDir })
  }

  async function uninstallPackage(id: string): Promise<void> {
    await invoke('uninstall_package', { id })
  }

  async function enablePlugin(id: string): Promise<void> {
    await invoke('enable_plugin', { id })
  }

  async function disablePlugin(id: string): Promise<void> {
    await invoke('disable_plugin', { id })
  }

  async function getPluginManifest(id: string) {
    return invoke('get_plugin_manifest', { id })
  }

  return {
    listPackages,
    installPackage,
    uninstallPackage,
    enablePlugin,
    disablePlugin,
    getPluginManifest,
  }
}
