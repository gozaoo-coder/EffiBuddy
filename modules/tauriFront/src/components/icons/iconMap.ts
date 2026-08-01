/**
 * 语义名 → Hugeicons 图标映射表
 *
 * 数据源：@hugeicons/core-free-icons（官方 Hugeicons 免费图标包，5400+ Stroke Rounded 图标，
 * 24×24 网格，MIT 协议）。每个图标本质是 [tag, attrs][] 数组，由 HugeIcon 渲染器转成 SVG。
 *
 * 设计说明：
 * - 保留项目原有「语义名」API（<Icon name="search" />），仅替换底层实现，消费方零改动。
 * - Hugeicons 免费包无 Chevron 系列，ArrowDown01/ArrowRight01/ArrowUp01 本身就是 V/>/^ 形，
 *   故 chevron-* 与 arrow-* 共用同一组图标。
 * - 免费包为 Stroke（描边）风格，star / pin-filled 等原「填充」语义统一映射到对应描边图标。
 * - 修复了旧实现因 icons/ 目录缺失导致的 menu / settings / loader / check 等图标渲染为空的问题。
 */
import {
  Menu01Icon,
  Cancel01Icon,
  Tick01Icon,
  Copy01Icon,
  Delete01Icon,
  Search01Icon,
  Add01Icon,
  MinusSignIcon,
  Settings02Icon,
  Edit01Icon,
  More01Icon,
  RefreshIcon,
  ReloadIcon,
  UndoIcon,
  CloudIcon,
  AiChat01Icon,
  AiChat02Icon,
  AiIdeaIcon,
  SmartPhone01Icon,
  Tablet01Icon,
  Watch01Icon,
  PuzzleIcon,
  InformationCircleIcon,
  GitMergeIcon,
  PowerIcon,
  KeyboardIcon,
  CornerDownLeftIcon,
  FullScreenIcon,
  DiscoverCircleIcon,
  Move01Icon,
  ArrowUp01Icon,
  ArrowDown01Icon,
  ArrowRight01Icon,
  ArrowLeft01Icon,
  StarIcon,
  ViewIcon,
  ViewOffIcon,
  Sun01Icon,
  Moon01Icon,
  ContrastIcon,
  PaintBoardIcon,
  PinIcon,
  Home01Icon,
  VolumeOffIcon,
  ExternalLinkIcon,
  GithubIcon,
  WechatIcon,
  QuoteUpIcon,
  Loading01Icon,
  BoltIcon,
  SparklesIcon,
  Robot01Icon,
  Brain01Icon,
  Wrench01Icon,
  Plug01Icon,
  Globe02Icon,
  Folder01Icon,
  File01Icon,
  Image01Icon,
  Camera01Icon,
  Mic01Icon,
  Attachment01Icon,
  Message01Icon,
  SentIcon,
  Book01Icon,
  Clock01Icon,
  AlarmClockIcon,
  Alert01Icon,
} from '@hugeicons/core-free-icons'

/** Hugeicons 图标数据类型（[tag, attrs][]），由所有图标导出共享。 */
export type IconData = typeof Search01Icon

/**
 * 语义名 → Hugeicons 图标。
 * 同一图标可挂多个语义别名，保证历史调用与新写法都能命中。
 */
export const iconMap: Record<string, IconData> = {
  // 菜单 / 关闭 / 勾选 / 复制
  menu: Menu01Icon,
  close: Cancel01Icon,
  cancel: Cancel01Icon,
  'cancel-circle': Cancel01Icon,
  check: Tick01Icon,
  'check-builtin': Tick01Icon,
  tick: Tick01Icon,
  copy: Copy01Icon,

  // 删除 / 搜索 / 增减
  delete: Delete01Icon,
  trash: Delete01Icon,
  remove: Delete01Icon,
  search: Search01Icon,
  magnify: Search01Icon,
  plus: Add01Icon,
  add: Add01Icon,
  create: Add01Icon,
  minus: MinusSignIcon,

  // 设置 / 编辑 / 更多
  settings: Settings02Icon,
  setting: Settings02Icon,
  edit: Edit01Icon,
  rename: Edit01Icon,
  pencil: Edit01Icon,
  more: More01Icon,
  'more-horizontal': More01Icon,

  // 刷新 / 同步 / 恢复
  refresh: RefreshIcon,
  reload: ReloadIcon,
  reboot: ReloadIcon,
  sync: ReloadIcon,
  restore: UndoIcon,
  undo: UndoIcon,

  // 云 / AI / 设备
  cloud: CloudIcon,
  ai: AiChat01Icon,
  'ai-on': AiChat02Icon,
  idea: AiIdeaIcon,
  device: SmartPhone01Icon,
  'device-pad': Tablet01Icon,
  'device-watch': Watch01Icon,
  tablet: Tablet01Icon,

  // 插件 / 信息 / 合并 / 电源 / 键盘
  puzzle: PuzzleIcon,
  plugin: PuzzleIcon,
  info: InformationCircleIcon,
  merge: GitMergeIcon,
  power: PowerIcon,
  'power-off': PowerIcon,
  keyboard: KeyboardIcon,
  'keyboard-mode': KeyboardIcon,
  enter: CornerDownLeftIcon,
  'return': CornerDownLeftIcon,

  // 全屏 / 发现 / 移动
  fullscreen: FullScreenIcon,
  discover: DiscoverCircleIcon,
  move: Move01Icon,

  // 方向（Hugeicons 免费包的 01 系列为 V/>/^ 形，兼作 chevron）
  'arrow-up': ArrowUp01Icon,
  'arrow-down': ArrowDown01Icon,
  'arrow-right': ArrowRight01Icon,
  'arrow-left': ArrowLeft01Icon,
  'chevron-up': ArrowUp01Icon,
  'chevron-down': ArrowDown01Icon,
  'chevron-right': ArrowRight01Icon,
  'chevron-left': ArrowLeft01Icon,

  // 评价 / 可见 / 主题
  star: StarIcon,
  'star-outline': StarIcon,
  eye: ViewIcon,
  view: ViewIcon,
  'eye-off': ViewOffIcon,
  'view-off': ViewOffIcon,
  sun: Sun01Icon,
  moon: Moon01Icon,
  auto: ContrastIcon,
  palette: PaintBoardIcon,

  // 标记 / 静音 / 链接 / 品牌
  pin: PinIcon,
  'pin-filled': PinIcon,
  home: Home01Icon,
  mute: VolumeOffIcon,
  'external-link': ExternalLinkIcon,
  github: GithubIcon,
  wechat: WechatIcon,

  // 引用 / 加载 / 闪电 / 火花
  quote: QuoteUpIcon,
  loader: Loading01Icon,
  loading: Loading01Icon,
  bolt: BoltIcon,
  spark: SparklesIcon,
  sparkles: SparklesIcon,

  // AI 角色 / 工具 / 插头 / 世界
  robot: Robot01Icon,
  thinking: Brain01Icon,
  brain: Brain01Icon,
  tool: Wrench01Icon,
  wrench: Wrench01Icon,
  plug: Plug01Icon,
  globe: Globe02Icon,

  // 文件 / 媒体 / 沟通 / 时间 / 警告
  folder: Folder01Icon,
  file: File01Icon,
  image: Image01Icon,
  camera: Camera01Icon,
  mic: Mic01Icon,
  attachment: Attachment01Icon,
  chat: Message01Icon,
  message: Message01Icon,
  send: SentIcon,
  sent: SentIcon,
  book: Book01Icon,
  clock: Clock01Icon,
  alarm: AlarmClockIcon,
  warning: Alert01Icon,
  alert: Alert01Icon,
}

/**
 * 按语义名解析图标数据。
 * @returns 命中返回图标数据；未命中返回 undefined（由调用方决定回退策略）
 */
export function resolveIcon(name: string): IconData | undefined {
  return iconMap[name]
}
