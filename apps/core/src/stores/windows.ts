/**
 * Windows store. Tracks which overlay windows are visible.
 * Cross-window sync happens via the Tauri event bus, not Pinia directly
 * (Pinia state is per-WebView).
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useWindowsStore = defineStore('windows', () => {
  const visible = ref<Record<string, boolean>>({
    dock: true,
    'widget-host': false,
    'package-store': false,
    settings: false,
  })

  function setVisible(label: string, v: boolean) {
    visible.value[label] = v
  }

  return { visible, setVisible }
})
