<script setup lang="ts">
/**
 * P2pPanel —— P2P 设备管理主面板（在 BindSheet 中展示，宽度 380px）
 *
 * 布局（自上而下）：
 * 1. 服务状态：启动状态点（脉冲动画）+ 本机 device_id（等宽字体）
 * 2. IP 配对：地址输入 + 角色选择 + 配对按钮
 * 3. 配对请求：嵌入 P2pPendingRequests，有请求时展开、无则收起（高度过渡）
 * 4. 设备列表：扫描按钮 + 设备卡片，按状态展示不同操作
 *
 * 状态全部来自 useP2p 单例；操作经 useToast 反馈。
 *
 * 动画全流程管线：
 * - 设备 / 请求项：TransitionGroup + anime.js，进入错峰淡入上移、离场高度坍缩
 * - 配对请求区块：Transition 包裹，展开 / 收起为高度 + 透明度过渡
 * - 服务状态点：启动时 ::after 伪元素脉冲扩散（CSS keyframe）
 * - 设备卡片：hover 时 border-color 过渡到 primary
 * - 按钮按压：复用基础 Button 组件自带的 anime.js scale 反馈
 */
import { ref, computed, onMounted } from 'vue'
import { animate } from 'animejs'
import { Button, SegmentedButton, useToast, type SegmentedOption } from './basic'
import P2pPendingRequests from './P2pPendingRequests.vue'
import { useP2p } from '../composables/useP2p'
import type { Device, DeviceStatus, PairRole } from '../types'

const { toast } = useToast()
const {
  status,
  devices,
  pendingRequests,
  loading,
  refreshAll,
  scan,
  pairByAddress,
  acceptPair,
  rejectPair,
  unpair,
  syncPull,
  syncPush,
} = useP2p()

// ── IP 配对表单 ──────────────────────────────────────────────
const address = ref('')
const role = ref<PairRole>('mirror')
const roleOptions: SegmentedOption[] = [
  { label: '镜像', value: 'mirror' },
  { label: '主机', value: 'host' },
  { label: '副本', value: 'replica' },
]

// ── 设备操作局部忙态 ─────────────────────────────────────────
// 在全局 loading 之上，定位到具体设备 / 动作，在对应按钮上显示 spinner
const busyId = ref<string | null>(null)
const busyKind = ref<'pair' | 'pull' | 'push' | 'unpair' | null>(null)
const busy = computed(() => loading.value || busyId.value !== null)
function clearBusy() {
  busyId.value = null
  busyKind.value = null
}

async function onPairByAddress() {
  const addr = address.value.trim()
  if (!addr || busy.value) return
  try {
    const dev = await pairByAddress(addr, role.value)
    if (dev) {
      toast({ content: `已与「${dev.name}」配对`, type: 'success' })
      address.value = ''
    } else {
      toast({ content: '配对失败', type: 'error' })
    }
  } catch (e) {
    toast({ content: `配对失败：${e}`, type: 'error' })
  }
}

async function onScan() {
  if (loading.value) return
  try {
    const found = await scan()
    toast({
      content: found.length ? `发现 ${found.length} 台设备` : '未发现新设备',
      type: found.length ? 'success' : 'info',
    })
  } catch (e) {
    toast({ content: `扫描失败：${e}`, type: 'error' })
  }
}

// 配对请求（来自子组件）
async function onAccept(deviceId: string, r: PairRole) {
  const ok = await acceptPair(deviceId, r)
  toast({ content: ok ? '配对成功' : '配对失败', type: ok ? 'success' : 'error' })
}
async function onReject(deviceId: string) {
  await rejectPair(deviceId)
  toast({ content: '已拒绝配对请求', type: 'info' })
}

// 设备卡片操作
async function onAcceptDiscovered(d: Device) {
  if (busy.value) return
  busyId.value = d.id
  busyKind.value = 'pair'
  const ok = await acceptPair(d.id, role.value)
  clearBusy()
  toast({ content: ok ? `已与「${d.name}」配对` : '配对失败', type: ok ? 'success' : 'error' })
}
async function onSyncPull(d: Device) {
  if (busy.value) return
  busyId.value = d.id
  busyKind.value = 'pull'
  const ok = await syncPull(d.id)
  clearBusy()
  toast({ content: ok ? `已从「${d.name}」拉取同步` : '同步拉取失败', type: ok ? 'success' : 'error' })
}
async function onSyncPush(d: Device) {
  if (busy.value) return
  busyId.value = d.id
  busyKind.value = 'push'
  const ok = await syncPush(d.id)
  clearBusy()
  toast({ content: ok ? `已向「${d.name}」推送同步` : '同步推送失败', type: ok ? 'success' : 'error' })
}
async function onUnpair(d: Device) {
  if (busy.value) return
  busyId.value = d.id
  busyKind.value = 'unpair'
  const ok = await unpair(d.id)
  clearBusy()
  toast({ content: ok ? `已取消与「${d.name}」的配对` : '取消配对失败', type: ok ? 'success' : 'error' })
}

// ── 状态文案 ──────────────────────────────────────────────
const statusLabels: Record<DeviceStatus, string> = {
  discovered: '已发现',
  paired: '在线',
  offline: '离线',
  pairing: '配对中',
}
function statusLabel(s: DeviceStatus): string {
  return statusLabels[s] ?? s
}

// ── 相对时间（兼容秒 / 毫秒时间戳） ──────────────────────
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

// ── 设备列表进出动画 ──────────────────────────────────────
function onItemBeforeEnter(el: Element) {
  const node = el as HTMLElement
  node.style.opacity = '0'
  node.style.transform = 'translateY(12px)'
}
function onItemEnter(el: Element, done: () => void) {
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
function onItemLeave(el: Element, done: () => void) {
  const node = el as HTMLElement
  const h = node.offsetHeight
  const mt = parseFloat(getComputedStyle(node).marginTop) || 0
  const mb = parseFloat(getComputedStyle(node).marginBottom) || 0
  node.style.height = `${h}px`
  node.style.marginTop = `${mt}px`
  node.style.marginBottom = `${mb}px`
  node.style.overflow = 'hidden'
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

// ── 配对请求区块展开 / 收起（高度 + 透明度过渡） ───────────
function onSectionBeforeEnter(el: Element) {
  const node = el as HTMLElement
  node.style.opacity = '0'
  node.style.height = '0px'
  node.style.overflow = 'hidden'
}
function onSectionEnter(el: Element, done: () => void) {
  const node = el as HTMLElement
  const target = node.scrollHeight
  animate(node, {
    opacity: [0, 1],
    height: [0, target],
    duration: 320,
    ease: 'out(3)',
    onComplete: () => {
      node.style.height = ''
      node.style.overflow = ''
      node.style.opacity = ''
      done()
    },
  })
}
function onSectionLeave(el: Element, done: () => void) {
  const node = el as HTMLElement
  const h = node.offsetHeight
  // overflow:hidden 在收起时裁剪内部子项的离场动画，只保留区块整体坍缩
  node.style.overflow = 'hidden'
  node.style.height = `${h}px`
  void node.offsetHeight
  animate(node, {
    opacity: [1, 0],
    height: [h, 0],
    duration: 280,
    ease: 'inOut(2)',
    onComplete: () => done(),
  })
}

onMounted(() => {
  // 静默刷新：refreshAll 内部已捕获状态拉取错误
  void refreshAll()
})
</script>

<template>
  <div class="p2p-panel">
    <!-- 1. 服务状态 -->
    <section class="p2p-status">
      <div class="status-row">
        <span class="status-dot" :class="{ 'is-on': status.started }"></span>
        <span class="status-text">{{ status.started ? 'P2P 服务已启动' : 'P2P 服务未启动' }}</span>
      </div>
      <div v-if="status.started && status.self_device_id" class="status-id">
        <span class="status-id-label">本机 ID</span>
        <code class="status-id-value" :title="status.self_device_id">{{ status.self_device_id }}</code>
      </div>
    </section>

    <!-- 2. IP / 链接配对（方法一） -->
    <section class="p2p-section">
      <h3 class="section-title">通过 IP 配对</h3>
      <div class="pair-form">
        <input
          v-model="address"
          class="pair-input"
          type="text"
          placeholder="例如 192.168.1.10:50051"
          :disabled="loading"
          @keyup.enter="onPairByAddress"
        />
        <SegmentedButton v-model="role" :options="roleOptions" size="sm" block />
        <Button
          variant="primary"
          size="md"
          block
          :loading="loading"
          :disabled="!address.trim()"
          @click="onPairByAddress"
        >配对</Button>
      </div>
    </section>

    <!-- 3. 配对请求（有请求时展开） -->
    <Transition
      :css="false"
      appear
      @before-enter="onSectionBeforeEnter"
      @enter="onSectionEnter"
      @leave="onSectionLeave"
    >
      <section v-if="pendingRequests.length > 0" class="p2p-section">
        <div class="section-head">
          <h3 class="section-title">配对请求</h3>
          <span class="section-count">{{ pendingRequests.length }}</span>
        </div>
        <P2pPendingRequests
          :requests="pendingRequests"
          :loading="loading"
          @accept="onAccept"
          @reject="onReject"
        />
      </section>
    </Transition>

    <!-- 4. 设备列表 -->
    <section class="p2p-section">
      <div class="section-head">
        <h3 class="section-title">已配对设备</h3>
        <Button variant="normal" size="sm" :loading="loading" @click="onScan">扫描</Button>
      </div>

      <div v-if="devices.length === 0" class="empty-hint">
        暂无设备，点击扫描发现局域网设备
      </div>
      <TransitionGroup
        v-else
        tag="div"
        class="device-list"
        :css="false"
        appear
        @before-enter="onItemBeforeEnter"
        @enter="onItemEnter"
        @leave="onItemLeave"
      >
        <div
          v-for="(d, i) in devices"
          :key="d.id"
          class="device-item"
          :data-id="d.id"
          :data-idx="i"
        >
          <div class="device-top">
            <span class="device-name" :title="d.name">{{ d.name }}</span>
            <span class="device-badge" :class="`device-badge--${d.status}`">{{ statusLabel(d.status) }}</span>
          </div>
          <div class="device-addr">{{ d.address }}</div>
          <div class="device-meta">
            <span class="device-time">上次在线 {{ formatRelativeTime(d.last_seen) }}</span>
          </div>
          <div class="device-actions">
            <template v-if="d.status === 'paired'">
              <Button variant="normal" size="sm" :disabled="busy" :loading="busyId === d.id && busyKind === 'pull'" @click="onSyncPull(d)">同步拉取</Button>
              <Button variant="normal" size="sm" :disabled="busy" :loading="busyId === d.id && busyKind === 'push'" @click="onSyncPush(d)">同步推送</Button>
              <Button variant="text" size="sm" :disabled="busy" @click="onUnpair(d)">取消配对</Button>
            </template>
            <template v-else-if="d.status === 'discovered'">
              <Button variant="primary" size="sm" :disabled="busy" :loading="busyId === d.id && busyKind === 'pair'" @click="onAcceptDiscovered(d)">配对</Button>
            </template>
            <template v-else-if="d.status === 'offline'">
              <Button variant="text" size="sm" :disabled="busy" @click="onUnpair(d)">取消配对</Button>
            </template>
            <template v-else>
              <Button variant="normal" size="sm" disabled>配对中</Button>
            </template>
          </div>
        </div>
      </TransitionGroup>
    </section>
  </div>
</template>

<style scoped>
.p2p-panel {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 16px;
}

/* ── 服务状态 ─────────────────────────────────────────── */
.p2p-status {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px;
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}
.status-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.status-dot {
  position: relative;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--muted);
  flex-shrink: 0;
  transition: background var(--duration-fast) var(--ease-standard);
}
.status-dot.is-on {
  background: var(--success);
}
.status-dot.is-on::after {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: 50%;
  background: var(--success);
  animation: p2p-pulse 2.4s var(--ease-standard) infinite;
}
@keyframes p2p-pulse {
  0% {
    transform: scale(1);
    opacity: 0.55;
  }
  100% {
    transform: scale(2.8);
    opacity: 0;
  }
}
.status-text {
  font-size: 13px;
  font-weight: 500;
  color: var(--text);
}
.status-id {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.status-id-label {
  font-size: 11px;
  color: var(--muted);
  white-space: nowrap;
}
.status-id-value {
  font-size: 12px;
  color: var(--text);
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ── 通用区块 ─────────────────────────────────────────── */
.p2p-section {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.section-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}
.section-count {
  min-width: 18px;
  height: 18px;
  padding: 0 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  font-weight: 600;
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 16%, transparent);
  border-radius: var(--radius-full);
}

/* ── IP 配对表单 ─────────────────────────────────────── */
.pair-form {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.pair-input {
  width: 100%;
  height: 36px;
  padding: 0 12px;
  font-size: 13px;
  color: var(--text);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
  box-sizing: border-box;
}
.pair-input::placeholder {
  color: var(--muted);
}
.pair-input:focus {
  border-color: var(--primary);
}
.pair-input:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

/* ── 设备列表 ─────────────────────────────────────────── */
.device-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.empty-hint {
  padding: 20px 14px;
  text-align: center;
  font-size: 12px;
  color: var(--muted);
  background: var(--card-2);
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
}
.device-item {
  padding: 12px 14px;
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  display: flex;
  flex-direction: column;
  gap: 8px;
  transition: border-color var(--duration-fast) var(--ease-standard);
}
.device-item:hover {
  border-color: var(--primary);
}
.device-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.device-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.device-addr {
  font-size: 12px;
  color: var(--muted);
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.device-meta {
  display: flex;
  align-items: center;
}
.device-time {
  font-size: 11px;
  color: var(--muted);
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
}
.device-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

/* ── 状态徽标 ─────────────────────────────────────────── */
.device-badge {
  flex-shrink: 0;
  font-size: 11px;
  padding: 2px 8px;
  border-radius: var(--radius-full);
  white-space: nowrap;
  border: 1px solid transparent;
}
.device-badge--discovered {
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 14%, transparent);
  border-color: color-mix(in srgb, var(--primary) 40%, transparent);
}
.device-badge--paired {
  color: var(--success);
  background: color-mix(in srgb, var(--success) 14%, transparent);
  border-color: color-mix(in srgb, var(--success) 40%, transparent);
}
.device-badge--offline {
  color: var(--muted);
  background: var(--card);
  border-color: var(--border);
}
.device-badge--pairing {
  color: var(--warn);
  background: color-mix(in srgb, var(--warn) 14%, transparent);
  border-color: color-mix(in srgb, var(--warn) 40%, transparent);
}
</style>
