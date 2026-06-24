/**
 * plugin-sdk-vue entry. Exports `definePlugin` helper and types.
 */
export { useCore, type CoreHandle } from './useCore'
export type {
  PluginFrontendModule,
  WidgetDeclaration,
} from './types'

import type { PluginFrontendModule } from './types'

/**
 * Type-safe helper to declare a plugin frontend module.
 * Just returns its argument; exists for editor autocomplete + future validation.
 */
export function definePlugin(module: PluginFrontendModule): PluginFrontendModule {
  return module
}
