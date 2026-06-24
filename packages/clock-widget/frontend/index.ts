/**
 * clock-widget frontend entry. Registers digital + analog widgets.
 */
import { definePlugin } from '@desktop-suite/plugin-sdk-vue'
import DigitalClock from './components/DigitalClock.vue'
import AnalogClock from './components/AnalogClock.vue'

export default definePlugin({
  id: 'com.desktopsuite.clock',
  widgets: [
    {
      type: 'digital',
      name: 'Digital Clock',
      defaultSize: { width: 240, height: 80 },
      component: DigitalClock,
    },
    {
      type: 'analog',
      name: 'Analog Clock',
      defaultSize: { width: 200, height: 200 },
      component: AnalogClock,
    },
  ],
  onEnable() {
    console.log('[clock-widget] frontend enabled')
  },
  onDisable() {
    console.log('[clock-widget] frontend disabled')
  },
})
