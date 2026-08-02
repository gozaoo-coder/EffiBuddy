/**
 * 图片全屏预览
 *
 * 点击消息内图片打开全屏预览(Teleport 到 body)。
 * 支持缩放(滚轮/按钮)、平移(拖拽)、旋转(按钮)、双击复位。
 * Esc 关闭,再次点击遮罩关闭。
 * window keydown 监听由 ImagePreview.vue 组件负责注册/卸载。
 */
import { reactive } from 'vue'

export function useImagePreview() {
  const previewState = reactive({
    visible: false,
    url: '',
    name: '',
    scale: 1,
    rotate: 0,
    tx: 0,
    ty: 0,
  })
  // 拖拽状态:pointerdown 记录起点,pointermove 更新 tx/ty
  const previewDrag = reactive({ active: false, startX: 0, startY: 0, baseTx: 0, baseTy: 0 })

  function resetPreviewTransform() {
    previewState.scale = 1
    previewState.rotate = 0
    previewState.tx = 0
    previewState.ty = 0
  }

  function openImagePreview(url: string, name: string) {
    if (!url) return
    previewState.url = url
    previewState.name = name
    resetPreviewTransform()
    previewState.visible = true
  }

  function closeImagePreview() {
    previewState.visible = false
    previewState.url = ''
    previewState.name = ''
    resetPreviewTransform()
  }

  function previewZoomIn() {
    previewState.scale = Math.min(previewState.scale * 1.25, 8)
  }
  function previewZoomOut() {
    previewState.scale = Math.max(previewState.scale / 1.25, 0.2)
  }
  function previewRotate() {
    previewState.rotate = (previewState.rotate + 90) % 360
  }

  function onPreviewWheel(e: WheelEvent) {
    e.preventDefault()
    if (e.deltaY < 0) previewZoomIn()
    else previewZoomOut()
  }

  function onPreviewPointerDown(e: PointerEvent) {
    // 仅主键(左键 / 触摸)触发拖拽
    if (e.button !== 0 && e.pointerType === 'mouse') return
    previewDrag.active = true
    previewDrag.startX = e.clientX
    previewDrag.startY = e.clientY
    previewDrag.baseTx = previewState.tx
    previewDrag.baseTy = previewState.ty
    ;(e.target as HTMLElement).setPointerCapture?.(e.pointerId)
  }

  function onPreviewPointerMove(e: PointerEvent) {
    if (!previewDrag.active) return
    previewState.tx = previewDrag.baseTx + (e.clientX - previewDrag.startX)
    previewState.ty = previewDrag.baseTy + (e.clientY - previewDrag.startY)
  }

  function onPreviewPointerUp(e: PointerEvent) {
    previewDrag.active = false
    ;(e.target as HTMLElement).releasePointerCapture?.(e.pointerId)
  }

  function onPreviewDblClick() {
    // 双击复位缩放与平移(保留旋转)
    previewState.scale = 1
    previewState.tx = 0
    previewState.ty = 0
  }

  function onPreviewKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') closeImagePreview()
    else if (e.key === '+' || e.key === '=') previewZoomIn()
    else if (e.key === '-') previewZoomOut()
    else if (e.key === '0') {
      previewState.scale = 1
      previewState.tx = 0
      previewState.ty = 0
    } else if (e.key === 'r') previewRotate()
  }

  return {
    previewState,
    resetPreviewTransform,
    openImagePreview,
    closeImagePreview,
    previewZoomIn,
    previewZoomOut,
    previewRotate,
    onPreviewWheel,
    onPreviewPointerDown,
    onPreviewPointerMove,
    onPreviewPointerUp,
    onPreviewDblClick,
    onPreviewKeydown,
  }
}
