/**
 * Window control composable. Wraps the window_* Tauri commands.
 */
import { useRustCommand } from './useRustCommand'

export function useWindow(currentLabel: string) {
  const invoke = useRustCommand()

  async function startDrag() {
    await invoke('start_dragging', { label: currentLabel })
  }

  async function showWindow(label: string) {
    await invoke('show_window', { label })
  }

  async function hideWindow(label: string = currentLabel) {
    await invoke('hide_window', { label })
  }

  async function closeWindow(label: string = currentLabel) {
    await invoke('close_window', { label })
  }

  async function setAlwaysOnTop(top: boolean, label: string = currentLabel) {
    await invoke('set_always_on_top', { label, top })
  }

  async function createWindow(opts: {
    label: string
    title: string
    url: string
    width: number
    height: number
  }) {
    await invoke('create_window', opts)
  }

  return { startDrag, showWindow, hideWindow, closeWindow, setAlwaysOnTop, createWindow }
}
