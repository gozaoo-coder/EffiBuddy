/**
 * Dock magnify effect. MVP: no-op stub returning hover handlers.
 * Real implementation will scale neighboring items based on cursor x.
 */
export function useMagnify() {
  function onHover(_id: string) {
    // TODO: scale neighbors per macOS Dock magnification.
  }
  function onLeave() {
    // TODO: reset scale.
  }
  return { onHover, onLeave }
}
