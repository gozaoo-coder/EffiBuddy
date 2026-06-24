/**
 * Plugin frontend interface types (mirror plugin-sdk-vue types).
 */
import type { Component } from 'vue'

export interface PluginWidget {
  type: string
  name: string
  defaultSize?: { width: number; height: number }
  component: Component
}

export interface PluginFrontendModule {
  id: string
  widgets?: PluginWidget[]
  settingsPage?: Component
  onEnable?: () => void | Promise<void>
  onDisable?: () => void | Promise<void>
}
