<script setup lang="ts">
/**
 * TabContent —— 页签内容容器
 *
 * 职责：
 * 1. 根据 activeTab 渲染对应页签组件（ChatTab / AsrStreamTab / AsrUploadTab / AsrHistoryTab）
 * 2. 用 KeepAlive 缓存已打开页签的组件状态（切换不丢消息 / 滚动位置 / 输入草稿）
 * 3. 页签切换时的进入 / 离开过渡动画
 * 4. 无页签时的空状态展示
 * 5. 透传 backend 给 chat 页签；向上冒泡 conversation-changed；内部消费 update:status
 *
 * 动画全流程管线（开始 / 中断 / 打断 / 返回 / DOM 进入退出 / 父子关系）：
 *
 * 采用「交叉淡入淡出 + 轻量 Y 位移」方案（非 out-in，避免串行延迟）：
 * 容器 position:relative；进入元素占正常流；离开元素 position:absolute 脱离流，
 * 使两者可短暂共存——新内容立即占据空间，旧内容在其上方淡出。
 *
 * 1. 进入（@before-enter → @enter）：
 *    - before-enter：插入后、首帧绘制前设置 opacity:0 / translateY(10px)，杜绝首帧闪现
 *    - 开始：activeTab 变化 → Transition 触发 @enter(el, done)
 *    - 动画：opacity 0→1 + translateY 10px→0，ease out(3)，duration 280ms
 *    - 中断：同一元素理论上仅进入一次；若残留未完成的 enter（极端竞态），先 pause 旧动画再起新
 *    - DOM：onComplete 清除内联 opacity/transform → done()
 *
 * 2. 离开（@leave）：
 *    - 开始：activeTab 变化 → 旧组件触发 @leave(el, done)
 *    - 打断：快速连续切换 A→B→C 时，B 的 enter 可能未完成即被 leave。
 *            先 pause B 的 enter 动画并清残留 transform，保证 leave 从自然态起步；
 *            再 pause 该元素上未完成的旧 leave（防御），起新 leave
 *    - 动画：opacity 1→0，duration 200ms（比进入快，减少视觉干扰），ease inOut(2)
 *    - DOM：离开元素 position:absolute; inset:0 脱离流；onComplete 清除全部离开态内联样式
 *           （position/inset/width/height/opacity），避免 KeepAlive 缓存复用时残留 → done()
 *
 * 3. 返回（切回已缓存页签）：
 *    - KeepAlive 命中缓存 → 组件实例不重建，仅触发 Transition 进入动画
 *    - 消息列表 / 滚动位置 / 输入草稿 / 计费统计全部保留
 *
 * 4. 父子关系：
 *    - TabContent（父，position:relative 容器，flex:1 撑满主区）
 *    - Transition（直接子，包裹 KeepAlive）
 *    - KeepAlive（孙，缓存粒度「组件类型 + :key(instanceKey)」）
 *    - 动态 component（曾孙，各 TabKind 对应组件 + 空状态组件）
 *
 * :key 取值 = tab.instanceKey（而非 tab.id）：
 *   chat 页签新建会话建立后，updateTab 会把 id 从 `__new_chat__` 迁移为真实 conversation_id。
 *   若用 id 作 :key，迁移瞬间组件重新挂载，流式传输中的 ChatWindow 实例被销毁、
 *   Tauri 事件监听丢失，首条消息流式中断。instanceKey 与 id 解耦，迁移 id 时实例不变。
 */
import { computed, type Component } from 'vue'
import { animate } from 'animejs'
import TabEmpty from './TabEmpty.vue'
import ChatTab from './tabs/ChatTab.vue'
import AsrStreamTab from './tabs/AsrStreamTab.vue'
import AsrUploadTab from './tabs/AsrUploadTab.vue'
  import AsrHistoryTab from './tabs/AsrHistoryTab.vue'
  import SubAgentTab from './tabs/SubAgentTab.vue'
  import PluginPageTab from './tabs/PluginPageTab.vue'
  import { useTabs } from '../composables/useTabs'
import type { TabItem } from '../types'

defineOptions({ name: 'TabContent' })

const props = defineProps<{
  /** 当前 agent 后端标识，仅 chat 页签需要 */
  backend?: string
}>()

const emit = defineEmits<{
  (e: 'conversation-changed'): void
}>()

const { getActive, updateTab } = useTabs()
const activeTab = getActive()

  const activeComponent = computed<Component>(() => {
    const t = activeTab.value
    if (!t) return TabEmpty
    switch (t.kind) {
      case 'chat':
        return ChatTab
      case 'sub-agent':
        return SubAgentTab
      case 'plugin':
        return PluginPageTab
      case 'asr-stream':
        return AsrStreamTab
      case 'asr-upload':
        return AsrUploadTab
      case 'asr-history':
        return AsrHistoryTab
    }
  })

// 稳定 key：instanceKey 优先；空状态用固定哨兵，保证切换可被 Transition 捕获
const activeKey = computed(() => activeTab.value?.instanceKey ?? '__empty__')

  const activeProps = computed<Record<string, unknown>>(() => {
    const t = activeTab.value
    if (!t) return {}
    return t.kind === 'chat' ? { tab: t, backend: props.backend } : { tab: t }
  })
// ============= 动画实例追踪（中断 / 打断用） =============
type Anim = ReturnType<typeof animate> | null
const enterAnims = new WeakMap<HTMLElement, NonNullable<Anim>>()
const leaveAnims = new WeakMap<HTMLElement, NonNullable<Anim>>()

// ============= Transition 钩子 =============
function onBeforeEnter(el: Element) {
  // 首帧前锁定初始态，杜绝插入后到动画起播前的可见闪现
  const htmlEl = el as HTMLElement
  htmlEl.style.opacity = '0'
  htmlEl.style.transform = 'translateY(10px)'
}

function onEnter(el: Element, done: () => void) {
  const htmlEl = el as HTMLElement
  // 中断：若该元素仍有未完成 enter（极端竞态），先 pause
  enterAnims.get(htmlEl)?.pause()
  const anim = animate(htmlEl, {
    opacity: [0, 1],
    translateY: ['10px', 0],
    duration: 280,
    ease: 'out(3)',
    onComplete: () => {
      htmlEl.style.opacity = ''
      htmlEl.style.transform = ''
      enterAnims.delete(htmlEl)
      done()
    },
  })
  enterAnims.set(htmlEl, anim)
}

function onLeave(el: Element, done: () => void) {
  const htmlEl = el as HTMLElement
  // 打断：若 enter 未完成（快速切换 A→B→C，B 刚进入即被切走），
  // 先 pause enter 并清残留 transform / opacity，保证 leave 从自然可见态起步
  const prevEnter = enterAnims.get(htmlEl)
  if (prevEnter) {
    prevEnter.pause()
    enterAnims.delete(htmlEl)
  }
  htmlEl.style.transform = ''
  htmlEl.style.opacity = ''
  // 脱离流：进入元素立即占据空间，离开元素在其上方淡出
  htmlEl.style.position = 'absolute'
  htmlEl.style.inset = '0'
  htmlEl.style.width = '100%'
  htmlEl.style.height = '100%'
  // 防御：pause 该元素上未完成的旧 leave
  leaveAnims.get(htmlEl)?.pause()
  const anim = animate(htmlEl, {
    opacity: [1, 0],
    duration: 200,
    ease: 'inOut(2)',
    onComplete: () => {
      // 清除全部离开态内联样式，避免 KeepAlive 缓存复用时残留 absolute 定位
      htmlEl.style.position = ''
      htmlEl.style.inset = ''
      htmlEl.style.width = ''
      htmlEl.style.height = ''
      htmlEl.style.opacity = ''
      leaveAnims.delete(htmlEl)
      done()
    },
  })
  leaveAnims.set(htmlEl, anim)
}

// ============= 事件处理 =============
// ChatWindow 新建会话后回传真实 conversation_id：
//   - __new_chat__ 哨兵 → 真实 id：迁移 tab.id + conversationId（instanceKey 不变，组件不重挂）
//   - 已存在会话页签：仅同步 conversationId 字段（防御性）
function onConvIdUpdate(newId: string | null) {
  const tab = activeTab.value
  if (!tab || tab.kind !== 'chat' || !newId) return
  if (newId === tab.id) {
    if (tab.conversationId !== newId) updateTab(tab.id, { conversationId: newId })
    return
  }
  // 迁移 id（useTabs.updateTab 处理目标 id 冲突合并 / activeTabId 同步）
  updateTab(tab.id, { id: newId, conversationId: newId })
  emit('conversation-changed')
}

function onConversationChanged() {
  emit('conversation-changed')
}

// 子页签上报状态（chat loading / asr recording 等）→ 写入 TabItem，供 TabBar 显示
function onStatusUpdate(status: TabItem['status']) {
  const tab = activeTab.value
  if (!tab) return
  if (tab.status !== status) updateTab(tab.id, { status })
}
</script>

<template>
  <div class="tab-content">
    <Transition :css="false" @before-enter="onBeforeEnter" @enter="onEnter" @leave="onLeave">
      <KeepAlive :max="40">
        <component
          :is="activeComponent"
          :key="activeKey"
          v-bind="activeProps"
          @update:conversation-id="onConvIdUpdate"
          @conversation-changed="onConversationChanged"
          @update:status="onStatusUpdate"
        />
      </KeepAlive>
    </Transition>
  </div>
</template>

<style scoped>
.tab-content {
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  background: var(--bg);
}

/* KeepAlive 包裹的动态组件根元素需填满容器，便于离开态 absolute 覆盖 */
.tab-content > :deep(*) {
  width: 100%;
  height: 100%;
}

</style>
