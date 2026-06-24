/**
 * Plugin frontend dynamic loader.
 *
 * MVP strategy: plugin frontends are pre-bundled into the Core build via
 * Vite's glob import (so `import()` works without a runtime asset server).
 * Each plugin's `frontend/index.ts` is mapped to a dynamic import path.
 *
 * For plugins installed at runtime (not present at build time), this falls
 * back to a placeholder component.
 */
import { defineComponent, h, type Component } from 'vue'

// Static map of plugin ids -> dynamic import factory.
// Built-in plugins are listed here; runtime-installed ones use the fallback.
const BUILTIN_LOADERS: Record<string, () => Promise<Record<string, unknown>>> = {
  'com.desktopsuite.clock': () => import('@plugins/clock-widget'),
}

export function usePluginFrontend() {
  async function loadWidget(pluginId: string, widgetType: string): Promise<Component | null> {
    const loader = BUILTIN_LOADERS[pluginId]
    if (!loader) {
      return fallbackWidget(pluginId, widgetType)
    }
    try {
      const mod = (await loader()) as {
        default?: { widgets?: Array<{ type: string; component: Component }> }
      }
      const plugin = mod.default
      if (!plugin?.widgets) return null
      const w = plugin.widgets.find((x) => x.type === widgetType)
      return w?.component ?? null
    } catch (e) {
      console.error('loadWidget failed', e)
      return fallbackWidget(pluginId, widgetType)
    }
  }

  function fallbackWidget(pluginId: string, widgetType: string): Component {
    return defineComponent({
      name: 'FallbackWidget',
      render() {
        return h('div', { class: 'fallback-widget' }, [
          h('div', { class: 'title' }, `${pluginId}`),
          h('div', { class: 'type' }, widgetType),
        ])
      },
    })
  }

  return { loadWidget }
}
