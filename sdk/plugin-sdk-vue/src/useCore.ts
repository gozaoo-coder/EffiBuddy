/**
 * useCore: composable giving plugin frontend components access to Core's
 * event bus and invoke bridge. Wraps Tauri event/listen APIs with a
 * plugin-scoped namespace.
 */
import { listen, emit, type UnlistenFn } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'

export interface CoreHandle {
  /** Listen to a Core event (auto-scoped to plugin id). */
  on: (event: string, handler: (payload: unknown) => void) => Promise<UnlistenFn>
  /** Emit an event to Core / other plugins. */
  emit: (event: string, payload?: unknown) => Promise<void>
  /** Invoke a Tauri command. */
  invoke: <T = unknown>(cmd: string, args?: Record<string, unknown>) => Promise<T>
}

export function useCore(pluginId: string): CoreHandle {
  return {
    async on(event, handler) {
      const topic = `plugin:${pluginId}:${event}`
      return listen<unknown>(topic, (e) => handler(e.payload))
    },
    async emit(event, payload) {
      const topic = `plugin:${pluginId}:${event}`
      await emit(topic, payload)
    },
    invoke(cmd, args) {
      return invoke<T>(cmd, args)
    },
  }
}
