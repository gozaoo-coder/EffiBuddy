/**
 * Settings store. Reads/writes the backend config store.
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { useRustCommand } from '@/composables/useRustCommand'

export const useSettingsStore = defineStore('settings', () => {
  const config = ref<Record<string, unknown>>({})
  const invoke = useRustCommand()

  async function load() {
    const raw = await invoke<Record<string, unknown>>('get_config', {})
    config.value = raw ?? {}
  }

  async function update(key: string, value: unknown) {
    await invoke('set_config', { key, value })
    config.value[key] = value
  }

  return { config, load, update }
})
