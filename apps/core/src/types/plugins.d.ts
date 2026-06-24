/**
 * Ambient declarations for built-in plugin frontend modules loaded via
 * Vite alias at runtime. The actual modules live under the packages
 * frontend folders and are not type-checked by Core tsconfig.
 */
declare module '@plugins/clock-widget' {
  const plugin: import('@desktop-suite/plugin-sdk-vue').PluginFrontendModule
  export default plugin
}
