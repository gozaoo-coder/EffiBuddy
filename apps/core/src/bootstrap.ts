/**
 * Shared app bootstrap: registers Pinia + VueTippy + loads persisted theme
 * before mount. Each window entry calls `setup(app)`.
 */
import type { App } from 'vue'
import { createPinia } from 'pinia'
import VueTippy from 'vue-tippy'
import 'tippy.js/dist/tippy.css'
import { useTheme } from '@/composables/useTheme'
import '@/assets/styles/global.css'

export function setup(app: App): App {
  app.use(createPinia())
  app.use(VueTippy, {
    defaultProps: { theme: 'ds', delay: [200, 0], duration: [150, 100] },
  })
  // Load theme before first paint.
  useTheme().load()
  return app
}
