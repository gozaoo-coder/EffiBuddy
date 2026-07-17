import { ref, watch, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { ThemeMode } from '../types'

// 全局主题状态（单例，多组件共享）
const themeMode = ref<ThemeMode>('system')
const resolvedTheme = ref<'light' | 'dark'>('dark')
let mediaQuery: MediaQueryList | null = null
let mqListener: ((e: MediaQueryListEvent) => void) | null = null

function applyResolved(theme: 'light' | 'dark') {
  resolvedTheme.value = theme
  document.documentElement.setAttribute('data-theme', theme)
}

function computeResolved(): 'light' | 'dark' {
  if (themeMode.value === 'system') {
    if (typeof window !== 'undefined' && window.matchMedia) {
      return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
    }
    return 'dark'
  }
  return themeMode.value
}

function syncFromMode() {
  applyResolved(computeResolved())
}

export function useTheme() {
  onMounted(async () => {
    // 读取后端持久化的主题
    try {
      const config = await invoke<{ theme: ThemeMode }>('get_config')
      themeMode.value = config.theme
    } catch {
      // 默认 system
    }

    // 监听系统主题变化（仅 system 模式生效）
    if (typeof window !== 'undefined' && window.matchMedia) {
      mediaQuery = window.matchMedia('(prefers-color-scheme: light)')
      mqListener = () => {
        if (themeMode.value === 'system') syncFromMode()
      }
      mediaQuery.addEventListener('change', mqListener)
    }

    syncFromMode()
  })

  onUnmounted(() => {
    if (mediaQuery && mqListener) {
      mediaQuery.removeEventListener('change', mqListener)
    }
  })

  async function setTheme(mode: ThemeMode) {
    themeMode.value = mode
    syncFromMode()
    try {
      await invoke('set_theme', { theme: mode })
    } catch (e) {
      console.warn('set_theme failed', e)
    }
  }

  return { themeMode, resolvedTheme, setTheme }
}

// 用于在 watch 外部手动触发（例如 App.vue 初始化时立即应用）
export function applyThemeNow(mode: ThemeMode) {
  themeMode.value = mode
  syncFromMode()
}
