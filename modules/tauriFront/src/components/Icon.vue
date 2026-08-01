<script setup lang="ts">
/**
 * Icon 图标组件
 *
 * 优先从 src/icons 目录加载 SVG（华为图标库），按语义名引用。
 * 对于 icons 目录中缺少的基础 UI 符号（close/check/chevron/star 等），
 * 提供内建 SVG path 作为补充，保证覆盖所有 UI 需求。
 *
 * 主题适配：SVG 的 fill/stroke 继承 currentColor，由父元素 color 控制。
 *
 * 用法：<Icon name="menu" :size="18" />
 */
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
    /** 语义名（如 menu / delete / search / close / check / chevron-down） */
    name: string
    /** 尺寸 px，默认 18 */
    size?: number | string
    /** 当 name 未匹配时显示的回退字符 */
    fallback?: string
  }>(),
  {
    size: 18,
    fallback: '',
  },
)

// 用 Vite 的 import.meta.glob 预加载 icons 目录所有 svg 的原始内容
// eager: true 同步加载，query: '?raw' 获取原始字符串
const iconModules = import.meta.glob('../icons/*.svg', {
  eager: true,
  query: '?raw',
  import: 'default',
}) as Record<string, string>

// icons 目录中 svg 文件名 → 原始 SVG 字符串
const iconFiles: Record<string, string> = {}
for (const [path, content] of Object.entries(iconModules)) {
  // 从路径提取文件名（不含扩展名），如 "../icons/ic_Delete.svg" → "ic_Delete"
  const name = path.split('/').pop()?.replace(/\.svg$/, '') ?? ''
  if (name) iconFiles[name] = content
}

// 语义名 → icons 目录文件名映射（通过 Grep path id 分析确定）
const SEMANTIC_MAP: Record<string, string> = {
  // 菜单
  menu: 'ic_celiakeyboard_menu',
  // 删除
  delete: 'ic_Delete',
  trash: 'ic_Delete',
  // 搜索/放大镜
  search: 'ic_celiakeyboard_menu', // 用 magnify 的 UUID 文件
  magnify: 'ic_celiakeyboard_menu',
  // 添加联系人
  'add-contact': 'ic_addcontact',
  'add-contact-filled': 'ic_addcontact_filled',
  // 加号/减号
  plus: 'ic_brightness_plus',
  minus: 'ic_brightness_reduce',
  // 设置
  settings: 'ic_gallery_set',
  // 编辑/重命名
  edit: 'ic_gallery_rename',
  rename: 'ic_gallery_rename',
  // 更多
  more: 'ic_gallery_photoedit_more',
  // 创建
  create: 'ic_gallery_create',
  // 勾选
  check: 'ic_gallery_material_select_checkbo',
  // 同步
  sync: 'ic_gallery_sync',
  // 云同步
  cloud: 'ic_cloud_synchronization',
  // AI
  ai: 'ic_ai_photography_normal',
  'ai-on': 'ic_ai_photography_on',
  // 设备
  device: 'ic_device_pad',
  'device-pad': 'ic_device_pad',
  'device-watch': 'ic_device_watch',
  // 插件/拼图
  puzzle: 'ic_puzzle',
  plugin: 'ic_puzzle',
  // 信息
  info: 'ic_privacy_statement',
  // 合并
  merge: 'ic_merge',
  // 电源
  power: 'ic_power_off',
  // 刷新/重启
  refresh: 'ic_reboot',
  // 键盘
  keyboard: 'ic_celiakeyboard_mode',
  // 回车
  enter: 'ic_celiakeyboard_enter',
  // 全屏
  fullscreen: 'ic_gallery_fullscreen',
  // 发现
  discover: 'ic_gallery_discover',
  // 免费
  free: 'ic_free',
  // 手写
  handwritten: 'ic_celiakeyboard_handwritten',
  // 恢复
  restore: 'ic_celiakeyboard_restore',
  // 移动
  move: 'ic_celiakeyboard_move',
}

// UUID 文件映射（有语义 id 但文件名是 UUID）
const UUID_MAP: Record<string, string> = {
  // ic_celiakeyboard_magnify → 搜索/放大镜
  search: '8d530877-faa2-4f3c-8ccc-055872322c53',
  magnify: '8d530877-faa2-4f3c-8ccc-055872322c53',
  // ic_celiakeyboard_menu_icon_size → 菜单
  'menu-icon': '7c97423f-2d48-4636-b717-fb5df7682df6',
  // ic_power_off → 电源
  'power-off': '7890d331-e758-47a7-a299-c044ee4079e3',
  // ic_reboot → 刷新
  reboot: '9877254e-f723-4f89-b71a-e5ea6cd90a5e',
  // ic_celiakeyboard_restore → 恢复
  'restore-icon': '7eafabb6-c328-4891-8d6c-6692a53f0e41',
  // ic_celiakeyboard_mode → 键盘模式
  'keyboard-mode': 'b7d39019-fb11-4644-a8c3-6c13a0175d33',
  // ic_desktop_widgets → 小部件
  widgets: '195125ea-c0b2-40e0-bf6b-ba1a4a6ace52',
  // ic_celiakeyboard_move → 移动
  'move-icon': '03558545-9c7f-4858-9408-375bd3783e55',
  // ic_celiakeyboard_mechanical → 机械键盘
  mechanical: 'ff265c85-4aeb-4ef5-90bc-8c80bf5b048d',
  // ic_celiakeyboard_resize → 调整大小
  resize: '899b5201-1a5e-4ea7-9e84-009e7cc46b38',
  // ic_celiakeyboard_swipe_up → 上滑
  'swipe-up': 'b1612251-4352-43db-9bbb-dda5e4b141ff',
  // ic_celiakeyboard_unfold_reverse → 展开/折叠
  'unfold-reverse': '7c1f2091-7102-4c82-8925-455055caf500',
  // 时钟（02b49aa4 通过 path 分析为时钟）
  clock: '02b49aa4-258d-4377-aa9f-294c57d30672',
  timer: '02b49aa4-258d-4377-aa9f-294c57d30672',
}

// icons 目录中缺少的基础 UI 符号，用内联 SVG path 补充
// 24x24 viewBox，与 icons 目录的 svg 一致
const BUILTIN_SVGS: Record<string, string> = {
  // 关闭 ×
  close: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M6.5 6.5L17.5 17.5M17.5 6.5L6.5 17.5" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 对勾 ✓
  'check-builtin': `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M5 12.5L10 17.5L19 6.5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  // 复制
  copy: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><rect x="8" y="8" width="12" height="12" rx="2" stroke="currentColor" stroke-width="2"/><path d="M16 8V6C16 5 15 4 14 4H6C5 4 4 5 4 6V14C4 15 5 16 6 16H8" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>`,
  // 向下箭头 ▾
  'chevron-down': `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M6 9L12 15L18 9" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  // 向右箭头 ▸/›
  'chevron-right': `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M9 6L15 12L9 18" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  // 向上箭头 ↑（发送）
  'arrow-up': `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M12 19L12 5M6 11L12 5L18 11" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  // 星 ★
  star: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M12 2L14.5 9H22L16 13.5L18.5 21L12 16.5L5.5 21L8 13.5L2 9H9.5L12 2Z"/></svg>`,
  'star-outline': `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M12 3.5L14.1 9.2L20 9.5L15.3 13.3L17 19L12 15.7L7 19L8.7 13.3L4 9.5L9.9 9.2L12 3.5Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/></svg>`,
  // 眼睛
  eye: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M2 12C2 12 6 5 12 5C18 5 22 12 22 12C22 12 18 19 12 19C6 19 2 12 2 12Z" stroke="currentColor" stroke-width="2"/><circle cx="12" cy="12" r="3.5" stroke="currentColor" stroke-width="2"/></svg>`,
  'eye-off': `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M3 3L21 21" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><path d="M10.5 10.7C10.2 11.1 10 11.5 10 12C10 13.1 10.9 14 12 14C12.5 14 12.9 13.8 13.3 13.5" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><path d="M9.4 5.7C10.2 5.3 11.1 5 12 5C18 5 22 12 22 12C21.1 13.5 20 14.8 18.8 15.8M6.2 7.3C3.7 9 2 12 2 12C2 12 6 19 12 19C12.9 19 13.7 18.8 14.5 18.5" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 太阳
  sun: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><circle cx="12" cy="12" r="4.5" stroke="currentColor" stroke-width="2"/><path d="M12 2V4M12 20V22M4 12H2M22 12H20M5 5L6.5 6.5M17.5 17.5L19 19M19 5L17.5 6.5M6.5 17.5L5 19" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 月亮
  moon: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M20 13.5C19.4 18 15.5 21 11.5 21C7 21 3 17 3 12C3 8 6 4.5 10 4C9.2 5.3 8.8 6.9 8.8 8.5C8.8 13 12.5 16.5 17 16.5C18 16.5 19 16.2 20 15.7" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  // 自动（跟随系统）
  auto: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="2"/><path d="M12 3V12L18 15" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 闪电
  bolt: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M13 2L4 14H11L10 22L20 9H13L13 2Z"/></svg>`,
  // 机器人
  robot: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><rect x="4" y="7" width="16" height="12" rx="3" stroke="currentColor" stroke-width="2"/><circle cx="9" cy="13" r="1.5" fill="currentColor"/><circle cx="15" cy="13" r="1.5" fill="currentColor"/><path d="M12 3V7" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><circle cx="12" cy="3" r="1.5" fill="currentColor"/><path d="M2 13V15M22 13V15" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 思考/大脑
  thinking: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M12 3C8.5 3 6 5.5 6 8.5C6 9 6.1 9.5 6.2 10C5 10.5 4 11.7 4 13C4 14.5 5 15.7 6.3 16C6.2 16.5 6 17 6 17.5C6 19.5 8 21 10.5 21H14C16.5 21 18.5 19.5 18.5 17C18.5 16.5 18.4 16 18.2 15.5C19.4 15 20 13.8 20 12.5C20 11.3 19.4 10.2 18.4 9.6C18.8 8.7 19 7.7 19 6.7C19 4.6 17 3 14.5 3H12Z" stroke="currentColor" stroke-width="1.8"/><circle cx="9.5" cy="12" r="0.8" fill="currentColor"/><circle cx="14.5" cy="12" r="0.8" fill="currentColor"/></svg>`,
  // 工具/扳手
  tool: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M14.5 4C12.5 4 11 5.5 11 7.5C11 7.8 11 8.1 11.1 8.4L4.5 15C3.5 16 3.5 17.5 4.5 18.5C5.5 19.5 7 19.5 8 18.5L14.6 11.9C14.9 12 15.2 12 15.5 12C17.5 12 19 10.5 19 8.5C19 7.8 18.8 7.1 18.4 6.5L15.5 9.4L14 8L16.9 5.1C16.1 4.4 15.3 4 14.5 4Z" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/></svg>`,
  // 麦克风
  mic: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><rect x="9" y="3" width="6" height="11" rx="3" stroke="currentColor" stroke-width="2"/><path d="M5 11C5 15 8 18 12 18C16 18 19 15 19 11M12 18V21" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 相机
  camera: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M3 8C3 7 4 6 5 6H7L8.5 4H15.5L17 6H19C20 6 21 7 21 8V18C21 19 20 20 19 20H5C4 20 3 19 3 18V8Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/><circle cx="12" cy="13" r="4" stroke="currentColor" stroke-width="2"/></svg>`,
  // 图片
  image: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><rect x="3" y="5" width="18" height="14" rx="2" stroke="currentColor" stroke-width="2"/><circle cx="8.5" cy="10.5" r="1.5" fill="currentColor"/><path d="M21 16L16 11L5 19" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>`,
  // 文件夹
  folder: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M3 6C3 5 4 4 5 4H9L11 6H19C20 6 21 7 21 8V18C21 19 20 20 19 20H5C4 20 3 19 3 18V6Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>`,
  // 文件
  file: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M6 3H14L19 8V19C19 20 18 21 17 21H6C5 21 4 20 4 19V5C4 4 5 3 6 3Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/><path d="M14 3V8H19" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>`,
  // 地球/网络
  globe: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="2"/><path d="M3 12H21M12 3C15 6 15 18 12 21M12 3C9 6 9 18 12 21" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 警告
  warning: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M12 3L22 20H2L12 3Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/><path d="M12 10V14M12 17V17.5" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 调色板
  palette: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M12 3C7 3 3 7 3 12C3 17 7 21 12 21C13.5 21 14.5 19.5 14 18C13.5 16.5 14.5 15 16 15H18C20 15 21 14 21 12C21 7 17 3 12 3Z" stroke="currentColor" stroke-width="2"/><circle cx="7" cy="11" r="1.3" fill="currentColor"/><circle cx="12" cy="8" r="1.3" fill="currentColor"/><circle cx="17" cy="11" r="1.3" fill="currentColor"/></svg>`,
  // 插头
  plug: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M9 3V8M15 3V8M7 8H17V12C17 14.8 14.8 17 12 17C9.2 17 7 14.8 7 12V8ZM12 17V21" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  // 聊天气泡
  chat: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M4 5C4 4 5 3 6 3H18C19 3 20 4 20 5V15C20 16 19 17 18 17H13L9 21V17H6C5 17 4 16 4 15V5Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>`,
  // 附件（回形针）
  attachment: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M16 6L8 14C7 15 7 16.5 8 17.5C9 18.5 10.5 18.5 11.5 17.5L18 11C19.5 9.5 19.5 7 18 5.5C16.5 4 14 4 12.5 5.5L6 12C4 14 4 17 6 19C8 21 11 21 13 19L19 13" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  // 定时器/闹钟
  alarm: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><circle cx="12" cy="13" r="8" stroke="currentColor" stroke-width="2"/><path d="M12 9V13L15 15M5 5L7 3M19 5L17 3" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 发送（纸飞机）
  send: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M22 2L11 13M22 2L15 22L11 13L2 9L22 2Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/></svg>`,
  // 三点（更多水平）
  'more-horizontal': `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><circle cx="5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="19" cy="12" r="1.8"/></svg>`,
  // Github
  github: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M12 2C6.5 2 2 6.5 2 12C2 16.4 5 20.1 9 21.4V18.5C8.5 18.6 8 18.5 7.8 18.2C7.5 17.6 7 17 6.5 16.9C6 16.8 5.8 16.5 6.2 16.5C7 16.4 7.7 17 8.3 17.7C8.8 18.3 9.3 18.4 9.8 18.2C9.9 17.7 10.1 17.3 10.4 17C8 16.7 6 15.5 6 12.3C6 11.3 6.3 10.5 6.8 9.9C6.7 9.7 6.5 8.8 6.9 7.7C6.9 7.7 7.6 7.5 9 8.4C9.6 8.2 10.3 8.1 11 8.1C11.7 8.1 12.4 8.2 13 8.4C14.4 7.5 15.1 7.7 15.1 7.7C15.5 8.8 15.3 9.7 15.2 9.9C15.7 10.5 16 11.3 16 12.3C16 15.5 14 16.7 11.6 17C11.9 17.3 12 17.7 12 18.2V21.4C16 20.1 19 16.4 19 12C19 6.5 14.5 2 19 2H12Z"/></svg>`,
  // 书/许可
  book: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M4 4C4 3 5 2 6 2H18C19 2 20 3 20 4V20C20 21 19 22 18 22H6C5 22 4 21 4 20V4Z" stroke="currentColor" stroke-width="2"/><path d="M8 6H16M8 10H16M8 14H13" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 外部链接
  'external-link': `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M14 5H19V10M19 5L10 14M16 14V19C16 20 15 21 14 21H6C5 21 4 20 4 19V11C4 10 5 9 6 9H11" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  // 置顶/图钉
  pin: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M9 4H15L14 10L17 13V14H12V20L12 14H7V13L10 10L9 4Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>`,
  'pin-filled': `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M9 4H15L14 10L17 13V14H12V20L12 14H7V13L10 10L9 4Z"/></svg>`,
  // 静音
  mute: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M11 5L7 9H4V15H7L11 19V5Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/><path d="M16 9L20 15M20 9L16 15" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 首页
  home: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M3 12L12 4L21 12V20C21 20.5 20.5 21 20 21H15V14H9V21H4C3.5 21 3 20.5 3 20V12Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>`,
  // 闪电（圆形）
  spark: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M13 2L4 14H11L10 22L20 9H13L13 2Z"/></svg>`,
  // 微信
  wechat: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M9 4C5 4 2 6.5 2 10C2 12 3 13.5 4.5 14.5L4 16L6 15C7 15.3 8 15.5 9 15.5C9.2 15.5 9.3 15.5 9.5 15.5C9.2 14.7 9 14 9 13.2C9 9.5 12.5 7 17 7C17.3 7 17.7 7 18 7.1C17.3 5.2 13.5 4 9 4ZM7 8C7.5 8 8 8.5 8 9C8 9.5 7.5 10 7 10C6.5 10 6 9.5 6 9C6 8.5 6.5 8 7 8ZM11 8C11.5 8 12 8.5 12 9C12 9.5 11.5 10 11 10C10.5 10 10 9.5 10 9C10 8.5 10.5 8 11 8ZM17 9C13.5 9 10.5 11 10.5 13.5C10.5 16 13.5 18 17 18C17.5 18 18 17.9 18.5 17.8L20 18.5L19.5 17C21 16.2 22 14.8 22 13.5C22 11 19 9 17 9ZM15 12C15.5 12 16 12.5 16 13C16 13.5 15.5 14 15 14C14.5 14 14 13.5 14 13C14 12.5 14.5 12 15 12ZM19 12C19.5 12 20 12.5 20 13C20 13.5 19.5 14 19 14C18.5 14 18 13.5 18 13C18 12.5 18.5 12 19 12Z"/></svg>`,
  // 引用（双引号）
  quote: `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M7 7C5 7 3.5 8.5 3.5 10.5C3.5 12.5 5 14 7 14C7.2 14 7.4 14 7.6 13.9C7.4 15.3 6.5 16.5 5 17L5.5 18.5C8 17.5 9.5 15 9.5 12V10.5C9.5 8.5 8.5 7 7 7ZM17 7C15 7 13.5 8.5 13.5 10.5C13.5 12.5 15 14 17 14C17.2 14 17.4 14 17.6 13.9C17.4 15.3 16.5 16.5 15 17L15.5 18.5C18 17.5 19.5 15 19.5 12V10.5C19.5 8.5 18.5 7 17 7Z"/></svg>`,
}

// 查找图标 SVG 内容
// 优先级：1. icons 目录语义映射 → 2. UUID 映射 → 3. 内建基础符号 → 4. 直接文件名 → 5. fallback
function resolveIcon(name: string): string | null {
  // 1. icons 目录语义映射
  const mapped = SEMANTIC_MAP[name]
  if (mapped && iconFiles[mapped]) return iconFiles[mapped]

  // 2. UUID 映射
  const uuidMapped = UUID_MAP[name]
  if (uuidMapped && iconFiles[uuidMapped]) return iconFiles[uuidMapped]

  // 3. 内建基础符号
  if (BUILTIN_SVGS[name]) return BUILTIN_SVGS[name]

  // 4. 直接用 name 作为文件名查找
  if (iconFiles[name]) return iconFiles[name]

  // 5. 未找到
  return null
}

// 处理 SVG：确保继承 currentColor
function processSvg(svg: string): string {
  // 移除 width/height 属性，让 CSS 控制
  let processed = svg.replace(/(<svg[^>]*?)\s+width="[^"]*"/i, '$1')
  processed = processed.replace(/(<svg[^>]*?)\s+height="[^"]*"/i, '$1')
  // 把 fill="#000" 或 fill="black" 等固定颜色替换为 currentColor（保留 fill="none"）
  processed = processed.replace(/\sfill="(?!none)[^"]*"/gi, ' fill="currentColor"')
  // 如果没有 fill 属性，添加 fill="currentColor"
  if (!/<svg[^>]*fill="/i.test(processed)) {
    processed = processed.replace(/<svg/i, '<svg fill="currentColor"')
  }
  return processed
}

const svgContent = computed(() => {
  const raw = resolveIcon(props.name)
  if (raw) return processSvg(raw)
  // fallback：返回一个空 span，显示 fallback 字符
  return null
})

const sizeStyle = computed(() => {
  const s = typeof props.size === 'number' ? `${props.size}px` : props.size
  return {
    width: s,
    height: s,
  }
})
</script>

<template>
  <span class="app-icon" :style="sizeStyle" v-html="svgContent" />
  <span v-if="!svgContent">{{ fallback }}</span>
</template>

<style scoped>
.app-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  vertical-align: middle;
  line-height: 0;
}

.app-icon :deep(svg) {
  width: 100%;
  height: 100%;
  display: block;
}
</style>
