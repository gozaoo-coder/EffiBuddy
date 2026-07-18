// 基础组件库统一导出
// 参考 HarmonyOS NEXT 设计规范，所有组件遵循 design tokens

// 图标
export { default as Icon } from '../Icon.vue'

// 反馈类
export { default as ToastHost } from './ToastHost.vue'
export { default as SnackbarHost } from './SnackbarHost.vue'
export { useToast, useSnackbar } from '../../composables/useFeedback'
export type { ToastType, ToastPosition, ToastOptions, SnackbarAction, SnackbarOptions } from '../../composables/useFeedback'

// 气泡提示
export { default as Popup } from './Popup.vue'
export type { PopupPlacement, PopupAlign, PopupTrigger, PopupButton } from './Popup.vue'

// 按钮组
export { default as Button } from './Button.vue'
export { default as ToggleButton } from './ToggleButton.vue'
export { default as IconButton } from './IconButton.vue'

// 下拉
export { default as Dropdown } from './Dropdown.vue'
export type { DropdownOption, DropdownSize } from './Dropdown.vue'

// 选择类
export { default as Switch } from './Switch.vue'
export { default as Radio } from './Radio.vue'
export { default as RadioGroup } from './RadioGroup.vue'
export { default as Slider } from './Slider.vue'
export { default as SegmentedButton } from './SegmentedButton.vue'
export type { SegmentedOption, SegmentedSize } from './SegmentedButton.vue'
export { default as Picker } from './Picker.vue'
export { default as Chips } from './Chips.vue'

// 容器类
export { default as Menu } from './Menu.vue'
export type { MenuItemOption, MenuPlacement, MenuSubMenuMode } from './Menu.vue'
export { useContextMenu } from './Menu.vue'
export { default as Dialog } from './Dialog.vue'
export { default as BindSheet } from './BindSheet.vue'

// 指示器
export { default as ContextRing } from './ContextRing.vue'
