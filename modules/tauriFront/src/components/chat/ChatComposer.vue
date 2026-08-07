<script setup lang="ts">
/**
 * ChatComposer —— 图一风格底部输入栏(卡片式复合输入框)
 *
 * 布局:圆角容器内上方为全宽 textarea,下方为底部操作栏
 * (+ 按钮 / meta pills / 右侧圆角方形发送按钮)。
 * 发送编排在 useChatSend(引用拼接 → 建会话 → 流式调用)实现,本组件只渲染 UI。
 *
 * 输入增强:
 * - `/` 命令面板:输入 / 唤起,匹配技能 / 插件 / 插件命令,Enter 或点击插入为提示词
 * - `@` 文件面板:输入 @ 唤起,基于工作区浏览文件 / 文件夹,目录可逐级进入,文件插入 @路径
 * - 拖放:把文件 / 文件夹拖入输入框,自动转成 @路径 引用
 */
import { ref, computed, inject, watch, nextTick } from 'vue'
import { animate } from 'animejs'
import { invoke } from '@tauri-apps/api/core'
import { Button, IconButton, Icon, Menu, type MenuItemOption } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'
import type {
  AgentConfig,
  AvailableModel,
  Skill,
  InstalledPlugin,
  DirEntryInfo,
  PluginContributionsAggregate,
  PluginCommandContribution,
} from '../../types'

const store = inject(CHAT_STORE_KEY)!

// 解构 ref:模板自动解包,script 中 .value 读写
const {
  input,
  sending,
  queuedCount,
  workingDir,
  thinking,
  reasoningEffort,
  workingDirSheetOpen,
  toolSheetOpen,
  shellBarExpanded,
  shellActiveCount,
  toggleShellBar,
  activeModelInfo,
  loadActiveModelInfo,
  toast,
} = store.core
const { quoteChips, scrollToMessage, removeQuote } = store.menu
const { compressBadgeInfo, compressSavedInfo, compressionSheetOpen } = store.compression
// 发送编排已抽到 useChatSend(core/streaming/menu/autoscroll 组合),
// 输入栏只保留 UI:渲染按钮状态 + 触发发送/停止。
const { send, stopGenerating } = store.send

const textareaRef = ref<HTMLTextAreaElement | null>(null)

// 发送后 input 被清空:回弹 textarea 高度到单行(useChatSend 不再关心 UI)
watch(input, (val, old) => {
  if (val === '' && old !== '') {
    const ta = textareaRef.value
    if (!ta) return
    ta.style.height = 'auto'
    const target = Math.min(ta.scrollHeight, 96)
    ta.style.height = target + 'px'
    void ta.offsetHeight
    animate(ta, {
      height: '44px',
      duration: 200,
      ease: 'out(3)',
    })
  }
})

// textarea 高度动画(关键:禁止 height: fit-content,用 animejs 动画)
function autoResize() {
  const ta = textareaRef.value
  if (!ta) return
  const currentHeight = ta.offsetHeight
  ta.style.height = 'auto'
  const naturalHeight = ta.scrollHeight
  ta.style.height = currentHeight + 'px'
  const targetHeight = Math.min(Math.max(naturalHeight, 44), 96)
  void ta.offsetHeight
  animate(ta, {
    height: targetHeight + 'px',
    duration: 200,
    ease: 'out(3)',
  })
}

// ---------- 键盘:面板打开时接管,否则 Enter 发送 ----------
function onKeydown(e: KeyboardEvent) {
  if (popupVisible.value) {
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        e.stopPropagation()
        moveActive(1)
        return
      case 'ArrowUp':
        e.preventDefault()
        e.stopPropagation()
        moveActive(-1)
        return
      case 'Enter':
      case 'Tab':
        e.preventDefault()
        e.stopPropagation()
        acceptActive()
        return
      case 'Escape':
        e.preventDefault()
        e.stopPropagation()
        closePopup()
        return
    }
    // 其余按键正常输入,由 onInput 重新分析触发
    return
  }
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault()
    void send()
  }
}

function onInput() {
  autoResize()
  void updatePopup()
}

// ---------- 触发检测:光标前最近 @(file) / 词首 /(command) ----------
type PopupMode = 'command' | 'file'

function getTrigger(): { mode: PopupMode; start: number; query: string } | null {
  const ta = textareaRef.value
  if (!ta) return null
  const pos = ta.selectionStart
  const before = input.value.slice(0, pos)
  // @ 文件触发:@ 前为行首或空白,其后至光标无空白(避免 email/提及误触)
  const atIdx = before.lastIndexOf('@')
  if (atIdx >= 0) {
    const prev = atIdx === 0 ? '' : before[atIdx - 1]
    const between = before.slice(atIdx + 1)
    if (!/\s/.test(between) && (atIdx === 0 || /\s/.test(prev))) {
      return { mode: 'file', start: atIdx, query: between }
    }
  }
  // / 命令触发:/ 前为行首或空白(避免 C:/xxx 绝对路径误触)
  const slashIdx = before.lastIndexOf('/')
  if (slashIdx >= 0) {
    const prev = slashIdx === 0 ? '' : before[slashIdx - 1]
    const between = before.slice(slashIdx + 1)
    if (!/\s/.test(between) && (slashIdx === 0 || /\s/.test(prev))) {
      return { mode: 'command', start: slashIdx, query: between }
    }
  }
  return null
}

// ---------- 弹出面板状态 ----------
interface PopupItem {
  id: string
  kind: string
  label: string
  desc?: string
  icon: string
  badge?: string
  payload?: unknown
}

const popupVisible = ref(false)
const popupMode = ref<PopupMode>('command')
const popupSections = ref<{ label: string; items: PopupItem[] }[]>([])
const activeIndex = ref(0)
const dragOver = ref(false)

const flatItems = computed(() => popupSections.value.flatMap((s) => s.items))
function flatIndex(gi: number, ii: number): number {
  let idx = 0
  for (let i = 0; i < gi; i++) idx += popupSections.value[i].items.length
  return idx + ii
}
function moveActive(d: number) {
  const n = flatItems.value.length
  if (n === 0) return
  activeIndex.value = (activeIndex.value + d + n) % n
}
function acceptActive() {
  const it = flatItems.value[activeIndex.value]
  if (it) applyPopupItem(it)
}
function closePopup() {
  popupVisible.value = false
  popupMode.value = 'command'
  activeIndex.value = 0
  popupSections.value = []
  fileEntries.value = []
  fileDirCache = ''
}

// 替换 [start, 光标) 区间为 text 并移动光标(程序化修改不触发 input 事件,调用方按需 updatePopup)
function replaceTokenAt(start: number, text: string) {
  const ta = textareaRef.value
  if (!ta) return
  const pos = ta.selectionStart
  const before = input.value.slice(0, start)
  const after = input.value.slice(pos)
  input.value = before + text + after
  nextTick(() => {
    ta.focus()
    const np = start + text.length
    ta.setSelectionRange(np, np)
  })
}

// ---------- 命令面板:技能 / 插件 / 插件命令 ----------
let skillsCache: Skill[] | null = null
let pluginsCache: InstalledPlugin[] | null = null
let cmdCache: { id: string; name: string; desc: string; plugin: string }[] | null = null

async function ensureCommandData() {
  if (skillsCache && pluginsCache && cmdCache) return
  const [skills, plugins, contribs] = await Promise.all([
    skillsCache ? Promise.resolve(skillsCache) : invoke<Skill[]>('list_skills'),
    pluginsCache
      ? Promise.resolve(pluginsCache)
      : invoke<InstalledPlugin[]>('list_installed_plugins'),
    cmdCache
      ? Promise.resolve(cmdCache)
      : invoke<PluginContributionsAggregate>('list_plugin_contributions').then((agg) => {
          const cmds: { id: string; name: string; desc: string; plugin: string }[] = []
          for (const p of agg.plugins ?? []) {
            for (const c of (p.commands ?? []) as PluginCommandContribution[]) {
              cmds.push({
                id: c.id,
                name: c.name,
                desc: c.description || '',
                plugin: p.displayName || p.pluginName,
              })
            }
          }
          return cmds
        }),
  ])
  skillsCache = skills
  pluginsCache = plugins
    cmdCache = contribs
}

function sourceLabel(s: Skill): string {
  if (s.builtin) return '内置'
  if (s.source === 'directory') return '目录'
  if (s.source === 'clawhub') return 'ClawHub'
  if (s.source === 'plugin') return '插件'
  return '本地'
}

function filterCommand(query: string) {
  const q = query.toLowerCase().trim()
  const sections: { label: string; items: PopupItem[] }[] = []

  const skills = (skillsCache ?? []).filter(
    (s) => !q || s.name.toLowerCase().includes(q) || s.description?.toLowerCase().includes(q),
  )
  if (skills.length) {
    sections.push({
      label: '技能',
      items: skills.map((s) => ({
        id: 'skill:' + s.id,
        kind: 'skill',
        label: s.name,
        desc: s.description,
        icon: 'sparkles',
        badge: sourceLabel(s),
        payload: s,
      })),
    })
  }

  const plugins = (pluginsCache ?? []).filter(
    (p) =>
      !q ||
      (p.display_name || p.name || '').toLowerCase().includes(q) ||
      (p.summary || '').toLowerCase().includes(q),
  )
  if (plugins.length) {
    sections.push({
      label: '插件',
      items: plugins.map((p) => ({
        id: 'plugin:' + p.id,
        kind: 'plugin',
        label: p.display_name || p.name,
        desc: p.summary || '',
        icon: 'plugin',
        payload: p,
      })),
    })
  }

  const cmds = (cmdCache ?? []).filter(
    (c) => !q || c.name.toLowerCase().includes(q) || c.desc.toLowerCase().includes(q),
  )
  if (cmds.length) {
    sections.push({
      label: '插件命令',
      items: cmds.map((c) => ({
        id: 'cmd:' + c.id,
        kind: 'cmd',
        label: c.name,
        desc: c.desc,
        icon: 'wrench',
        badge: c.plugin,
        payload: c,
      })),
    })
  }

  popupSections.value = sections
  activeIndex.value = 0
}

// ---------- 文件面板:@ 浏览 / 匹配文件与文件夹 ----------
let fileDirCache = ''
const fileEntries = ref<DirEntryInfo[]>([])

function isAbs(p: string): boolean {
  return /^[A-Za-z]:[\\/]/.test(p) || p.startsWith('/') || p.startsWith('\\\\')
}

// @ 后 token 拆分为 目录部分 + 文件名部分
function parseFileToken(token: string): { dir: string; name: string } {
  const idx = Math.max(token.lastIndexOf('/'), token.lastIndexOf('\\'))
  if (idx < 0) return { dir: '', name: token }
  return { dir: token.slice(0, idx + 1), name: token.slice(idx + 1) }
}

function resolveBaseDir(dirPart: string): string {
  if (!dirPart) return workingDir?.value || '~'
  if (dirPart.startsWith('~')) return dirPart
  if (isAbs(dirPart)) return dirPart
  const wd = workingDir?.value
  if (wd) {
    const sep = wd.includes('\\') ? '\\' : '/'
    return wd.endsWith('/') || wd.endsWith('\\') ? wd + dirPart : wd + sep + dirPart
  }
  return dirPart
}

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}

async function ensureFileData(query: string) {
  const { dir } = parseFileToken(query)
  const base = resolveBaseDir(dir)
  if (fileDirCache === base) return
  fileDirCache = base
  try {
    fileEntries.value = await invoke<DirEntryInfo[]>('list_directory', { dir: base })
  } catch {
    fileEntries.value = []
  }
}

function filterFile(query: string) {
  const { name } = parseFileToken(query)
  const q = name.toLowerCase()
  const items = fileEntries.value
    .filter((en) => !q || en.name.toLowerCase().includes(q))
    .map((en) => ({
      id: 'file:' + en.path,
      kind: en.is_dir ? 'dir' : 'file',
      label: en.name + (en.is_dir ? ' /' : ''),
      desc: en.is_dir ? '文件夹' : (en.extension ? en.extension + ' · ' : '') + fmtSize(en.size),
      icon: en.is_dir ? 'folder' : 'file',
      payload: en,
    }))
  popupSections.value = [{ label: '', items }]
  activeIndex.value = 0
}

// 目录项进入:把 @ 后路径替换为 @<dir>/ 并重新加载
function enterDir(dirPath: string) {
  const t = getTrigger()
  if (!t) return
  const sep = dirPath.includes('\\') ? '\\' : '/'
  replaceTokenAt(t.start, '@' + dirPath + sep)
  void updatePopup()
}

// 统一弹出面板更新入口(由 onInput / 进入目录调用)
async function updatePopup() {
  const t = getTrigger()
  if (!t) {
    closePopup()
    return
  }
  if (t.mode === 'command') {
    popupMode.value = 'command'
    if (!popupVisible.value || !skillsCache) popupVisible.value = true
    await ensureCommandData()
    filterCommand(t.query)
    if (!popupVisible.value) popupVisible.value = true
  } else {
    popupMode.value = 'file'
    if (!popupVisible.value) popupVisible.value = true
    await ensureFileData(t.query)
    filterFile(t.query)
  }
}

// 面板项确认:命令→插入提示词;文件→进入目录或插入 @路径
function applyPopupItem(item: PopupItem) {
  if (popupMode.value === 'file') {
    const entry = item.payload as DirEntryInfo
    if (entry?.is_dir) {
      enterDir(entry.path)
      return
    }
    replaceTokenAt(getTrigger()?.start ?? 0, '@' + entry.path)
    closePopup()
    return
  }
  const start = getTrigger()?.start ?? 0
  if (item.kind === 'skill') replaceTokenAt(start, `启用技能 ${item.label}`)
  else if (item.kind === 'plugin') replaceTokenAt(start, `打开插件 ${item.label}`)
  else if (item.kind === 'cmd') replaceTokenAt(start, `执行命令 ${item.label}`)
  closePopup()
}

// ---------- 拖放文件 / 文件夹(转成 @路径 引用) ----------
function onDragOver(e: DragEvent) {
  e.preventDefault()
  dragOver.value = true
}
function onDragLeave() {
  dragOver.value = false
}
function onDrop(e: DragEvent) {
  e.preventDefault()
  dragOver.value = false
  const files = e.dataTransfer?.files
  if (!files || files.length === 0) return
  const paths: string[] = []
  for (let i = 0; i < files.length; i++) {
    const f = files[i] as File & { path?: string }
    if (f.path) paths.push(f.path)
  }
  if (paths.length === 0) return
  const ta = textareaRef.value
  const pos = ta?.selectionStart ?? input.value.length
  const before = input.value.slice(0, pos)
  const after = input.value.slice(pos)
  const sep = before && !/\s$/.test(before) ? ' ' : ''
  const text = paths.map((p) => `@${p}`).join(' ')
  const nv = before + sep + text + ' ' + after
  input.value = nv
  nextTick(() => {
    if (ta) {
      const np = before.length + sep.length + text.length + 1
      ta.focus()
      ta.setSelectionRange(np, np)
    }
  })
}

// ---------- 推理设置菜单（思考开关 + reasoning_effort 等级） ----------
const reasoningMenuVisible = ref(false)
const reasoningBtnRef = ref<HTMLElement | null>(null)

const effortLabels: Record<'low' | 'high' | 'max', string> = {
  low: '低',
  high: '高',
  max: '顶级',
}

/** pill 文案:关闭 → 「思考已关」;开启 → 「思考·等级」 */
const reasoningLabel = computed(() =>
  thinking.value ? `思考·${effortLabels[reasoningEffort.value]}` : '思考已关',
)

const reasoningItems = computed<MenuItemOption[]>(() => [
  { key: 'off', label: '关闭思考', selected: !thinking.value },
  {
    key: 'low',
    label: '低',
    selected: thinking.value && reasoningEffort.value === 'low',
    divided: true,
  },
  { key: 'high', label: '高', selected: thinking.value && reasoningEffort.value === 'high' },
  { key: 'max', label: '顶级', selected: thinking.value && reasoningEffort.value === 'max' },
])

function onReasoningSelect(item: MenuItemOption) {
  if (item.key === 'off') {
    thinking.value = false
    return
  }
  thinking.value = true
  reasoningEffort.value = item.key as 'low' | 'high' | 'max'
}

// ---------- 当前对话模型选择菜单 ----------
const modelMenuVisible = ref(false)
const modelBtnRef = ref<HTMLElement | null>(null)
const chatModels = ref<AvailableModel[]>([])

const modelItems = computed<MenuItemOption[]>(() =>
  chatModels.value.map((m) => ({
    key: m.id,
    label: m.label,
    selected: activeModelInfo?.value?.id === m.id,
  })),
)

/** 打开菜单时按需拉取对话模型列表（get_config 过滤 kind=chat） */
async function toggleModelMenu() {
  modelMenuVisible.value = !modelMenuVisible.value
  if (!modelMenuVisible.value) return
  try {
    const cfg = await invoke<AgentConfig>('get_config')
    chatModels.value = cfg.models.filter((m) => (m.kind ?? 'chat') === 'chat')
  } catch (e) {
    console.warn('load models failed', e)
  }
}

async function onModelSelect(item: MenuItemOption) {
  if (activeModelInfo?.value?.id === item.key) return
  try {
    await invoke('set_active_model', { id: item.key })
    await loadActiveModelInfo()
    toast({ content: `已切换模型：${item.label}`, type: 'success' })
  } catch (e) {
    toast({ content: `切换模型失败：${e}`, type: 'error' })
  }
}

</script>

<template>
  <!-- 图一风格底部输入栏:卡片式复合输入框 -->
  <div
    class="composer"
    @dragover.prevent="onDragOver"
    @dragleave="onDragLeave"
    @drop.prevent="onDrop"
  >
    <!-- 拖放高亮遮罩 -->
    <div v-if="dragOver" class="composer-drop-overlay">
      <Icon name="attachment" :size="20" />
      <span>松开以附加文件 / 文件夹</span>
    </div>

    <!-- / 命令面板 + @ 文件面板(向上弹出) -->
    <div
      v-if="popupVisible"
      class="composer-popup"
      @mousedown.prevent
      @mouseleave="dragOver = false"
    >
      <div class="composer-popup-head">
        <span class="composer-popup-title">
          {{ popupMode === 'file' ? '选择文件 / 文件夹' : '命令面板' }}
        </span>
        <span v-if="popupMode === 'file'" class="composer-popup-path" :title="fileDirCache">
          {{ fileDirCache || '~' }}
        </span>
      </div>
      <div class="composer-popup-list">
        <template v-for="(sec, gi) in popupSections" :key="gi">
          <div v-if="sec.label && popupMode === 'command'" class="composer-popup-group">
            {{ sec.label }}
          </div>
          <div
            v-for="(it, ii) in sec.items"
            :key="it.id"
            class="composer-popup-item"
            :class="{ 'is-active': flatIndex(gi, ii) === activeIndex }"
            @mousedown.prevent.stop="activeIndex = flatIndex(gi, ii); applyPopupItem(it)"
          >
            <Icon :name="it.icon" :size="14" />
            <span class="composer-popup-item-label">{{ it.label }}</span>
            <span v-if="it.desc" class="composer-popup-item-desc">{{ it.desc }}</span>
            <span v-if="it.badge" class="composer-popup-item-badge">{{ it.badge }}</span>
          </div>
        </template>
        <div v-if="!flatItems.length" class="composer-popup-empty">无匹配项</div>
      </div>
      <div class="composer-popup-foot">
        <template v-if="popupMode === 'command'">↑↓ 选择 · Enter 插入 · Esc 关闭</template>
        <template v-else>目录 Enter 进入 · 文件 Enter 插入 · 支持 ~ / 绝对路径</template>
      </div>
    </div>

    <!-- 引用块区 -->
    <div v-if="quoteChips.length" class="quote-chips">
      <div
        v-for="q in quoteChips"
        :key="q.messageId"
        class="quote-chip"
        @click="scrollToMessage(q.messageId)"
      >
        <Icon name="quote" :size="12" />
        <span class="quote-chip-text">{{ q.snippet }}</span>
        <button
          type="button"
          class="quote-chip-close"
          title="移除引用"
          @click.stop="removeQuote(q.messageId)"
        >
          <Icon name="close" :size="12" />
        </button>
      </div>
    </div>

    <!-- composer-container 包裹层:上方输入区 + 底部操作栏 -->
    <div class="composer-container">
      <textarea
        ref="textareaRef"
        v-model="input"
        class="composer-input"
        :placeholder="
          sending
            ? queuedCount > 0
              ? `生成中…可继续输入（已排队 ${queuedCount} 条，将插入下一轮）`
              : '生成中…可继续输入（将插入下一轮）'
            : '随便问点什么…（/ 命令 · @ 文件 · 可拖入文件）'
        "
        rows="2"
        @keydown="onKeydown"
        @input="onInput"
      ></textarea>

      <!-- 底部操作栏:+ 按钮 + meta pills + 右侧发送按钮 -->
      <div class="composer-actions">
        <IconButton size="sm" container title="附件" @click="toolSheetOpen = true">
          <Icon name="plus" :size="16" />
        </IconButton>
        <!-- 推理设置:点击弹出 Menu 选择思考开关与 reasoning_effort 等级 -->
        <button
          ref="reasoningBtnRef"
          type="button"
          class="meta-pill meta-pill--reasoning"
          :class="{ 'meta-pill--reasoning-on': thinking }"
          title="推理设置（思考开关 / 推理强度）"
          @click="reasoningMenuVisible = !reasoningMenuVisible"
        >
          <Icon name="thinking" :size="12" />
          <span class="meta-pill-text">{{ reasoningLabel }}</span>
          <Icon :name="reasoningMenuVisible ? 'chevron-down' : 'chevron-up'" :size="11" />
        </button>
        <!-- 当前对话模型选择:显示激活模型名,点击弹出 Menu 切换(set_active_model 热替换 agent) -->
        <button
          ref="modelBtnRef"
          type="button"
          class="meta-pill meta-pill--model"
          :title="activeModelInfo ? `当前对话模型：${activeModelInfo.name}（点击切换）` : '选择对话模型'"
          @click="toggleModelMenu"
        >
          <Icon name="robot" :size="12" />
          <span class="meta-pill-text meta-pill-text--ellipsis">
            {{ activeModelInfo?.name ?? '未设置模型' }}
          </span>
          <Icon :name="modelMenuVisible ? 'chevron-down' : 'chevron-up'" :size="11" />
        </button>
        <button
          type="button"
          class="meta-pill meta-pill--wd"
          :title="workingDir ?? '未设置'"
          @click="workingDirSheetOpen = true"
        >
          <Icon name="folder" :size="12" />
          <span class="meta-pill-text meta-pill-text--ellipsis">
            {{ workingDir ? workingDir : '默认工作区' }}
          </span>
        </button>
        <!-- 生成中排队指示:AI 生成期间用户发送的消息将在下一轮插入 -->
        <button
          v-if="queuedCount > 0"
          type="button"
          class="meta-pill meta-pill--queued"
          :title="`${queuedCount} 条消息将在 AI 的下一个回复轮次前插入`"
        >
          <Icon name="clock" :size="12" />
          <span class="meta-pill-text">已排队 {{ queuedCount }} 条</span>
        </button>
        <!-- 压缩状态徽章:仅当当前会话已有压缩状态时显示,点击跳到压缩浮窗 -->
        <button
          v-if="compressBadgeInfo"
          type="button"
          class="meta-pill meta-pill--compress"
          :title="`当前会话已压缩 ${compressBadgeInfo.count} 条消息（第 ${compressBadgeInfo.level} 级 · ${compressBadgeInfo.actionCount} 条决策）${compressSavedInfo && compressSavedInfo.savedTokens > 0 ? ` · 节省约 ${compressSavedInfo.savedTokens} tokens` : ''} · 点击查看`"
          @click="compressionSheetOpen = true"
        >
          <Icon name="merge" :size="12" />
          <span class="meta-pill-text">已压缩 {{ compressBadgeInfo.count }}<template v-if="compressBadgeInfo.level > 0">·L{{ compressBadgeInfo.level }}</template><template v-if="compressSavedInfo && compressSavedInfo.savedTokens > 0">·↓{{ compressSavedInfo.savedTokens }}</template></span>
        </button>
        <!-- 命令会话折叠开关:展开/收起底部 ShellSessionBar(实时展示 AI 的 shell 工作状态) -->
        <button
          type="button"
          class="meta-pill meta-pill--ss"
          :class="{ 'meta-pill--ss-on': shellBarExpanded }"
          :title="shellBarExpanded ? '折叠命令会话栏' : '展开命令会话栏'"
          @click="toggleShellBar()"
        >
          <Icon name="keyboard" :size="12" />
          <span class="meta-pill-text">
            命令会话
            <span v-if="shellActiveCount > 0" class="meta-pill-badge">{{ shellActiveCount }}</span>
          </span>
          <Icon :name="shellBarExpanded ? 'chevron-down' : 'chevron-up'" :size="11" />
        </button>
        <!-- 右侧占位:把发送/停止按钮推到操作栏最右 -->
        <div class="composer-actions-spacer" />

        <!-- AI 生成中:右侧按钮变为红色「停止生成」(点击取消当前流) -->
        <Button
          v-if="sending"
          icon-only
          size="sm"
          variant="danger"
          class="composer-send"
          title="停止生成"
          @click="stopGenerating"
        >
          <template #icon><Icon name="stop" :size="16" /></template>
        </Button>
        <Button
          v-else
          icon-only
          size="sm"
          :variant="input.trim() ? 'primary' : 'normal'"
          :disabled="!input.trim()"
          class="composer-send"
          title="发送（Enter）"
          @click="send"
        >
          <template #icon><Icon name="arrow-up" :size="16" /></template>
        </Button>
      </div>
    </div>

    <!-- 对话模型选择菜单:位于输入栏上方弹出 -->
    <Menu
      v-model:visible="modelMenuVisible"
      :items="modelItems"
      :trigger-ref="modelBtnRef"
      title="对话模型"
      placement="top-start"
      :min-width="180"
      @select="onModelSelect"
    />

    <!-- 推理设置菜单:位于输入栏上方弹出 -->
    <Menu
      v-model:visible="reasoningMenuVisible"
      :items="reasoningItems"
      :trigger-ref="reasoningBtnRef"
      title="推理设置"
      placement="top-start"
      :min-width="140"
      @select="onReasoningSelect"
    />
  </div>
</template>

<style scoped>
.composer {
  position: relative;
}

.composer-input {
  width: 100%;
  resize: none;
  min-height: 44px;
  max-height: 96px;
  padding: 4px 6px 2px;
  font-family: inherit;
  font-size: 13px;
  line-height: 1.45;
  color: var(--text);
  background: transparent;
  border: none;
  outline: none;
}

.composer-input::placeholder {
  color: var(--muted);
}

.composer-input:disabled {
  opacity: 0.7;
  cursor: not-allowed;
}

/* ---------- 引用块 ---------- */
.quote-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding: 0 2px 6px;
}

.quote-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  max-width: 280px;
  padding: 3px 8px 3px 9px;
  font-size: 11px;
  color: var(--text);
  background: color-mix(in srgb, var(--primary) 8%, var(--card));
  border: 1px solid color-mix(in srgb, var(--primary) 24%, var(--border));
  border-radius: var(--radius-full);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease;
}

.quote-chip:hover {
  background: color-mix(in srgb, var(--primary) 14%, var(--card));
  border-color: var(--primary);
}

.quote-chip-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.quote-chip-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  flex-shrink: 0;
}

.quote-chip-close:hover {
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 10%, transparent);
}

/* ---------- 拖放遮罩 ---------- */
.composer-drop-overlay {
  position: absolute;
  inset: 0;
  z-index: 5;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  font-size: 13px;
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 8%, var(--card-2));
  border: 1.5px dashed color-mix(in srgb, var(--primary) 60%, var(--border));
  border-radius: var(--radius-lg);
  pointer-events: none;
}

/* ---------- / 命令面板 + @ 文件面板 ---------- */
.composer-popup {
  position: absolute;
  left: 0;
  right: 0;
  bottom: calc(100% + 8px);
  z-index: 60;
  display: flex;
  flex-direction: column;
  max-height: 300px;
  overflow: hidden;
  background: color-mix(in srgb, var(--card) 92%, transparent);
  backdrop-filter: blur(18px) saturate(1.4);
  -webkit-backdrop-filter: blur(18px) saturate(1.4);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.18);
}

.composer-popup-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-bottom: 1px solid var(--border);
}

.composer-popup-title {
  font-size: 11px;
  font-weight: 700;
  color: var(--text);
  flex-shrink: 0;
}

.composer-popup-path {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 10px;
  color: var(--muted);
  text-align: right;
  direction: rtl;
}

.composer-popup-list {
  overflow-y: auto;
  padding: 4px;
  flex: 1;
}

.composer-popup-group {
  padding: 5px 8px 2px;
  font-size: 10px;
  font-weight: 600;
  color: var(--muted);
}

.composer-popup-item {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 5px 8px;
  border-radius: var(--radius-md);
  cursor: pointer;
  font-size: 12px;
  color: var(--text);
}

.composer-popup-item.is-active {
  background: color-mix(in srgb, var(--primary) 14%, transparent);
}

.composer-popup-item-label {
  flex-shrink: 0;
  font-weight: 600;
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.composer-popup-item-desc {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
  color: var(--muted);
}

.composer-popup-item-badge {
  flex-shrink: 0;
  padding: 1px 6px;
  font-size: 9px;
  border-radius: var(--radius-full);
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 12%, transparent);
}

.composer-popup-empty {
  padding: 14px;
  text-align: center;
  font-size: 12px;
  color: var(--muted);
}

.composer-popup-foot {
  padding: 5px 10px;
  border-top: 1px solid var(--border);
  font-size: 10px;
  color: var(--muted);
}

/* ---------- composer 升级 ---------- */
/* composer-container 包裹层:亮色浅灰,暗色用 --card-2;紧凑排版 */
.composer-container {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 10px 6px;
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}

[data-theme='light'] .composer-container {
  background: var(--card-2);
}

.composer.focused .composer-container {
  border-color: color-mix(in srgb, var(--primary) 50%, var(--border));
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 12%, transparent);
}

/* ---------- 底部操作栏:+ 按钮 / meta pills / 右侧发送按钮 ---------- */
.composer-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-wrap: wrap;
  padding: 0 2px;
}

.composer-actions-spacer {
  flex: 1;
  min-width: 8px;
}

/* 右侧发送/停止按钮:圆角方形,与操作栏高度对齐 */
.composer-send {
  flex-shrink: 0;
}

/* 推理设置 pill:开启思考时用 primary 收敛色高亮 */
.meta-pill--reasoning-on {
  color: var(--primary);
}

/* 模型选择 pill:主角色收敛色,突出当前模型 */
.meta-pill--model {
  color: var(--primary);
  max-width: 220px;
}

.meta-pill--model:hover {
  color: var(--primary);
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
  background: color-mix(in srgb, var(--primary) 8%, transparent);
}

.meta-pill--reasoning-on:hover,
.meta-pill--reasoning:hover {
  color: var(--primary);
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
  background: color-mix(in srgb, var(--primary) 8%, transparent);
}

.meta-pill {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  color: var(--muted);
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--radius-full);
  cursor: pointer;
  transition: color 0.15s ease, background 0.15s ease, border-color 0.15s ease;
}

.meta-pill:hover {
  color: var(--text);
  background: var(--bg-2);
  border-color: var(--border);
}

.meta-pill--wd {
  max-width: 200px;
}

/* 压缩状态徽章:仅当会话已压缩时显示,配色用 success 收敛色 */
.meta-pill--compress {
  color: var(--success);
}

.meta-pill--compress:hover {
  color: var(--success);
  border-color: color-mix(in srgb, var(--success) 30%, var(--border));
  background: color-mix(in srgb, var(--success) 8%, transparent);
}

[data-theme='light'] .meta-pill--compress {
  background: rgba(16, 163, 127, 0.08);
}

/* 生成中排队指示:AI 生成期间用户发送的消息将在下一轮插入 */
.meta-pill--queued {
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 30%, var(--border));
  background: color-mix(in srgb, var(--warn) 10%, transparent);
}

.meta-pill--queued:hover {
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 45%, var(--border));
  background: color-mix(in srgb, var(--warn) 14%, transparent);
}
/* 命令会话折叠开关:激活(展开)态用 primary 收敛色,徽标显示运行中数量 */
.meta-pill--ss:hover,
.meta-pill--ss-on {
  color: var(--primary);
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
  background: color-mix(in srgb, var(--primary) 8%, transparent);
}

.meta-pill--ss-on:hover {
  color: var(--primary);
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
  background: color-mix(in srgb, var(--primary) 8%, transparent);
}

.meta-pill-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 14px;
  height: 14px;
  padding: 0 4px;
  border-radius: var(--radius-full);
  font-size: 9px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: var(--success);
  background: color-mix(in srgb, var(--success) 14%, transparent);
}

/* 会话版本管理入口已移除(composer 不再提供入口) */

.meta-pill-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.meta-pill-text--ellipsis {
  max-width: 180px;
}
</style>
