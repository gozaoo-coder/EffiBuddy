/**
 * Frontend plugin registration types.
 * A plugin frontend exports a `PluginFrontendModule` via its entry file.
 */
import type { Component } from 'vue'

export interface WidgetDeclaration {
  /** Widget type id, unique within the plugin. */
  type: string
  /** Display name shown in the widget picker. */
  name: string
  /** Default size in px. */
  defaultSize?: { width: number; height: number }
  /** Vue component implementing the widget. */
  component: Component
}

export interface PluginFrontendModule {
  /** Plugin id, must match manifest.json `id`. */
  id: string
  /** Widgets provided by this plugin. */
  widgets?: WidgetDeclaration[]
  /** Optional settings page component. */
  settingsPage?: Component
  /** Called when plugin is enabled in frontend. */
  onEnable?: () => void | Promise<void>
  /** Called when plugin is disabled in frontend. */
  onDisable?: () => void | Promise<void>
}
