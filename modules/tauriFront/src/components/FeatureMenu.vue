<script setup lang="ts">
/**
 * FeatureMenu —— 功能菜单（左栏一重定义）
 *
 * 左栏一（原 IconRail 常驻栏）从 layout 分离，封装为下拉功能菜单，
 * 由 TitleBar 左侧第一个按钮点击弹出（dropdown menu 方式）。
 *
 * 内容（图标 + 文字，默认展开态）：
 * - 主区：聊天 / Agent 交流池（活跃角标）/ 模型配置 / 自动化 / 技能 / 插件 /
 *         桌面小组件 / 语音转写（子菜单）/ 插件贡献按钮
 * - 底部：P2P 设备（待配对角标）/ 更多（子菜单：ClawHub / 设置）
 *
 * 已移除：「更多」菜单中的「修改侧栏icon」入口（IconRailSettings 一并删除）。
 * 选中项以 ✓ 高亮（active 视图）。
 */
import { computed, onMounted, ref, watch } from 'vue'
import { Menu, type MenuItemOption } from './basic'
import { useRailItems } from '../composables/useRailItems'
import { usePluginContributions } from '../composables/usePluginContributions'
import type { RailView } from '../types'

const props = withDefaults(
  defineProps<{
    /** 是否显示（v-model） */
    visible?: boolean
    /** 触发元素引用（TitleBar 第一个按钮），用于菜单定位 */
    triggerRef?: HTMLElement | null
    /** 当前激活的视图（用于 ✓ 高亮对应项） */
    active?: RailView | ''
    /** P2P 待配对请求计数（>0 时在 P2P 项显示气泡角标） */
    pendingPairCount?: number
    /** 交流池活跃条目数（>0 时在交流池项显示气泡角标） */
    poolActiveCount?: number
  }>(),
  {
    visible: false,
    triggerRef: null,
    active: '',
    pendingPairCount: 0,
    poolActiveCount: 0,
  },
)

const emit = defineEmits<{
  (e: 'update:visible', v: boolean): void
  (e: 'select', view: RailView): void
  (e: 'open-plugin-page', pageId: string): void
  (e: 'open-plugin-command', commandId: string): void
  (e: 'open-clawhub'): void
  (e: 'open-p2p'): void
  (e: 'open-settings'): void
  (e: 'open-asr', kind: 'asr-stream' | 'asr-upload' | 'asr-history'): void
}>()

const { allItems, mainItems, bottomItems, iconFor } = useRailItems()
const { railButtons, install: installPluginContributions } = usePluginContributions()
const { setPluginContributions } = useRailItems()

/** 插件 rail 贡献 → 注册表（响应式注入，与旧 IconRail 行为一致） */
onMounted(() => {
  void installPluginContributions()
})
watch(
  () => railButtons.value,
  (btns) => setPluginContributions(btns),
  { immediate: true },
)

// ============= 子菜单 =============
const asrItems: MenuItemOption[] = [
  { key: 'asr-stream', label: '流式录入', icon: 'mic' },
  { key: 'asr-upload', label: '文件转写', icon: 'attachment' },
  { key: 'asr-history', label: '历史记录', icon: 'clock' },
]

const moreItems: MenuItemOption[] = [
  { key: 'clawhub', label: 'ClawHub 技能市场', icon: 'globe' },
  { key: 'settings', label: '设置', icon: 'settings' },
]

// key → RailItemDef 映射（select 时反查行为）
const itemsByKey = computed(() => {
  const map = new Map<string, (typeof allItems.value)[number]>()
  for (const item of allItems.value) map.set(item.key, item)
  return map
})

function badgeText(count: number): string | undefined {
  if (count <= 0) return undefined
  return count > 99 ? '99+' : String(count)
}

/** 功能菜单项列表（图标 + 文字） */
const items = computed<MenuItemOption[]>(() => {
  const list: MenuItemOption[] = []

  // 主区：view / asr（子菜单）/ plugin
  for (const item of mainItems.value) {
    if (item.kind === 'view') {
      list.push({
        key: item.key,
        label: item.label,
        icon: iconFor(item.key),
        selected: props.active === item.value,
        badge: item.key === 'pool' ? badgeText(props.poolActiveCount) : undefined,
      })
    } else if (item.kind === 'asr') {
      list.push({ key: item.key, label: item.label, icon: iconFor(item.key), children: asrItems })
    } else if (item.kind === 'plugin') {
      list.push({ key: item.key, label: item.label, icon: iconFor(item.key) })
    }
  }

  // 底部组：P2P + 更多（首项带分隔线）
  const bottom = bottomItems.value.filter((i) => i.kind === 'p2p' || i.kind === 'more')
  bottom.forEach((item, idx) => {
    if (item.kind === 'p2p') {
      list.push({
        key: item.key,
        label: item.label,
        icon: iconFor(item.key),
        divided: idx === 0,
        badge: badgeText(props.pendingPairCount),
      })
    } else if (item.kind === 'more') {
      list.push({
        key: item.key,
        label: item.label,
        icon: iconFor(item.key),
        divided: idx === 0,
        children: moreItems,
      })
    }
  })

  return list
})

function onSelect(item: MenuItemOption) {
  emit('update:visible', false)
  const def = itemsByKey.value.get(item.key)
  if (def) {
    switch (def.kind) {
      case 'view':
        emit('select', def.value as RailView)
        break
      case 'plugin':
        if (def.pageId) emit('open-plugin-page', def.pageId)
        else if (def.command) emit('open-plugin-command', def.command)
        break
      case 'p2p':
        emit('open-p2p')
        break
    }
    return
  }
  // 子菜单项
  if (item.key.startsWith('asr-')) {
    emit('open-asr', item.key as 'asr-stream' | 'asr-upload' | 'asr-history')
  } else if (item.key === 'clawhub') {
    emit('open-clawhub')
  } else if (item.key === 'settings') {
    emit('open-settings')
  }
}
</script>

<template>
  <Menu
    v-model:visible="props.visible"
    :items="items"
    :trigger-ref="props.triggerRef"
    title="功能菜单"
    placement="bottom-start"
    :min-width="224"
    :position-offset="{ y: 4 }"
    @select="onSelect"
  />
</template>
