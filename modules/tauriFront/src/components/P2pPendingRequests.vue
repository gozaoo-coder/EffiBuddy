<script setup lang="ts">
/**
 * P2pPendingRequests —— P2P 待处理配对请求列表（子组件）
 *
 * 职责：
 * - 展示来自 useP2p 的 pendingRequests，每条含名称 / 地址 / 相对时间
 * - 提供角色选择（mirror / host / replica）与 接受 / 拒绝 操作，事件上抛父级
 * - 列表增删由 anime.js v4 驱动：进入错峰淡入上移、离场高度坍缩
 *
 * 动画全流程管线：
 * - 进入：beforeEnter 锁定初始态(opacity0 / translateY12) 避免首帧闪烁；
 *   enter 以 index*60ms 错峰、350ms outQuad 完成；onComplete 清理内联样式
 * - 离场：读取真实高度 / 外边距 → overflow:hidden → 同步坍缩到 0，
 *   下方兄弟随布局回流平滑上移（无需 FLIP transform）
 * - 打断：anime.js 每次调用覆盖同元素上一帧目标，新动画自然接管旧动画
 */
import { ref, watch } from 'vue'
import { animate } from 'animejs'
import { Button, SegmentedButton, type SegmentedOption } from './basic'
import type { PairingRequest, PairRole } from '../types'

const props = defineProps<{
  requests: PairingRequest[]
  loading: boolean
}>()

const emit = defineEmits<{
  (e: 'accept', deviceId: string, role: PairRole): void
  (e: 'reject', deviceId: string): void
}>()

// 配对角色：默认 mirror，接受时随事件上抛
const role = ref<PairRole>('mirror')
const roleOptions: SegmentedOption[] = [
  { label: '镜像', value: 'mirror' },
  { label: '主机', value: 'host' },
  { label: '副本', value: 'replica' },
]

// 当前操作中的请求 id 与动作类型：用于在对应按钮上显示 spinner
const busyId = ref<string | null>(null)
const busyKind = ref<'accept' | 'reject' | null>(null)

function onAccept(r: PairingRequest) {
  if (props.loading) return
  busyId.value = r.device_id
  busyKind.value = 'accept'
  emit('accept', r.device_id, role.value)
}
function onReject(r: PairingRequest) {
  if (props.loading) return
  busyId.value = r.device_id
  busyKind.value = 'reject'
  emit('reject', r.device_id)
}
// 父级异步完成后 loading 归 false，清忙
watch(
  () => props.loading,
  (v) => {
    if (!v) {
      busyId.value = null
      busyKind.value = null
    }
  },
)

// ── 相对时间格式化（兼容秒 / 毫秒时间戳） ──────────────────────
function formatRelativeTime(ts: number): string {
  if (!ts) return ''
  const ms = ts > 1e12 ? ts : ts * 1000
  const diff = Date.now() - ms
  if (diff < 0) return '刚刚'
  const sec = Math.floor(diff / 1000)
  if (sec < 60) return '刚刚'
  const min = Math.floor(sec / 60)
  if (min < 60) return `${min} 分钟前`
  const hr = Math.floor(min / 60)
  if (hr < 24) return `${hr} 小时前`
  const day = Math.floor(hr / 24)
  if (day < 7) return `${day} 天前`
  return new Date(ms).toLocaleDateString('zh-CN')
}

// ── 列表进出动画（anime.js v4 + TransitionGroup JS 钩子） ────────
function onBeforeEnter(el: Element) {
  const node = el as HTMLElement
  node.style.opacity = '0'
  node.style.transform = 'translateY(12px)'
}
function onEnter(el: Element, done: () => void) {
  const node = el as HTMLElement
  const idx = Number(node.dataset.idx ?? '0')
  const delay = Math.min(idx, 8) * 60
  animate(node, {
    opacity: [0, 1],
    translateY: [12, 0],
    duration: 350,
    delay,
    ease: 'outQuad',
    onComplete: () => {
      node.style.transform = ''
      node.style.opacity = ''
      done()
    },
  })
}
function onLeave(el: Element, done: () => void) {
  const node = el as HTMLElement
  const h = node.offsetHeight
  const mt = parseFloat(getComputedStyle(node).marginTop) || 0
  const mb = parseFloat(getComputedStyle(node).marginBottom) || 0
  node.style.height = `${h}px`
  node.style.marginTop = `${mt}px`
  node.style.marginBottom = `${mb}px`
  node.style.overflow = 'hidden'
  // 强制 reflow，使设定的固定高度生效后再向 0 坍缩
  void node.offsetHeight
  animate(node, {
    opacity: [1, 0],
    height: [h, 0],
    marginTop: [mt, 0],
    marginBottom: [mb, 0],
    duration: 280,
    ease: 'inOut(2)',
    onComplete: () => done(),
  })
}
</script>

<template>
  <div class="p2p-pending">
    <template v-if="requests.length > 0">
      <div class="pending-role">
        <span class="pending-role-label">配对角色</span>
        <SegmentedButton v-model="role" :options="roleOptions" size="sm" />
      </div>

      <TransitionGroup
        tag="div"
        class="pending-list"
        :css="false"
        appear
        @before-enter="onBeforeEnter"
        @enter="onEnter"
        @leave="onLeave"
      >
        <div
          v-for="(r, i) in requests"
          :key="r.device_id"
          class="pending-item"
          :data-id="r.device_id"
          :data-idx="i"
        >
          <div class="pending-top">
            <span class="pending-name" :title="r.name">{{ r.name }}</span>
            <span class="pending-badge">待处理</span>
          </div>
          <div class="pending-addr">{{ r.address }}</div>
          <div class="pending-bottom">
            <span class="pending-time">{{ formatRelativeTime(r.timestamp) }}</span>
            <div class="pending-actions">
              <Button
                variant="normal"
                size="sm"
                :disabled="loading"
                :loading="busyId === r.device_id && busyKind === 'reject'"
                @click="onReject(r)"
              >拒绝</Button>
              <Button
                variant="primary"
                size="sm"
                :disabled="loading"
                :loading="busyId === r.device_id && busyKind === 'accept'"
                @click="onAccept(r)"
              >接受</Button>
            </div>
          </div>
        </div>
      </TransitionGroup>
    </template>
    <div v-else class="pending-empty">暂无配对请求</div>
  </div>
</template>

<style scoped>
.p2p-pending {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.pending-role {
  display: flex;
  align-items: center;
  gap: 8px;
}
.pending-role-label {
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
}

.pending-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.pending-item {
  padding: 12px 14px;
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  display: flex;
  flex-direction: column;
  gap: 8px;
  transition: border-color var(--duration-fast) var(--ease-standard);
}
.pending-item:hover {
  border-color: color-mix(in srgb, var(--primary) 50%, transparent);
}

.pending-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.pending-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pending-badge {
  flex-shrink: 0;
  font-size: 11px;
  padding: 2px 8px;
  border-radius: var(--radius-full);
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 14%, transparent);
  border: 1px solid color-mix(in srgb, var(--primary) 40%, transparent);
  white-space: nowrap;
}

.pending-addr {
  font-size: 12px;
  color: var(--muted);
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pending-bottom {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.pending-time {
  font-size: 11px;
  color: var(--muted);
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
}
.pending-actions {
  display: flex;
  gap: 8px;
}

.pending-empty {
  padding: 16px;
  text-align: center;
  font-size: 12px;
  color: var(--muted);
}
</style>
