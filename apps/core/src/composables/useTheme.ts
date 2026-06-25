/**
 * Theme composable: system / dark / light. Mirrors OS color scheme via
 * `prefers-color-scheme` media query and persists choice to backend config.
 */
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { useRustCommand } from './useRustCommand'

export type ThemeMode = 'system' | 'dark' | 'light'

const mode = ref<ThemeMode>('system')
const systemDark = ref(false)
let mq: MediaQueryList | null = null

function applyTheme() {
  const dark = mode.value === 'dark' || (mode.value === 'system' && systemDark.value)
  document.documentElement.setAttribute('data-theme', dark ? 'dark' : 'light')
}

export function useTheme() {
  const invoke = useRustCommand()

  async function load() {
    const stored = await invoke<string>('get_config', { key: 'theme' }).catch(() => null)
    if (stored === 'dark' || stored === 'light' || stored === 'system') {
      mode.value = stored
    }
    applyTheme()
  }

  async function set(next: ThemeMode) {
    mode.value = next
    await invoke('set_config', { key: 'theme', value: next }).catch(() => {})
    applyTheme()
  }

  onMounted(() => {
    mq = window.matchMedia('(prefers-color-scheme: dark)')
    systemDark.value = mq.matches
    mq.addEventListener('change', onSystemChange)
    applyTheme()
  })
  onUnmounted(() => {
    mq?.removeEventListener('change', onSystemChange)
  })

  function onSystemChange(e: MediaQueryListEvent) {
    systemDark.value = e.matches
    applyTheme()
  }

  watch(mode, applyTheme)

  return { mode, set, load }
}
