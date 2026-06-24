/**
 * Typed wrapper around Tauri `invoke`.
 */
import { invoke, type InvokeArgs } from '@tauri-apps/api/core'

export function useRustCommand() {
  return function invokeCmd<T = unknown>(cmd: string, args?: InvokeArgs): Promise<T> {
    return invoke<T>(cmd, args as Record<string, unknown>)
  }
}
