<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { animate, stagger } from 'animejs'
import { Button, useToast } from './basic'
import type { Device, DeviceStatus, DeviceFoundPayload, DeviceStatusChangedPayload } from '../types'

const { toast } = useToast()

const devices = ref<Device[]>([])
const scanning = ref(false)
const pairingId = ref<string | null>(null)

let unlistenFound: UnlistenFn | null = null
let unlistenStatus: UnlistenFn | null = null

// 合并设备：按 id 去重，新数据覆盖旧数据。
function mergeDevices(incoming: Device[]) {
  const map = new Map<string, Device>()
  for (const d of devices.value) map.set(d.id, d)
  for (const d of incoming) map.set(d.id, d)
  devices.value = [...map.values()]
}

// 设备列表出现时的错峰入场动画。
async function animateList() {
  if (devices.value.length === 0) return
  await nextTick()
  const targets = document.querySelectorAll('.device-item')
  if (targets.length === 0) return
  animate('.device-item', {
    opacity: [0, 1],
    translateY: [12, 0],
    delay: stagger(60),
    duration: 350,
    easing: 'easeOutQuad',
  })
}

async function loadDevices() {
  try {
    devices.value = await invoke<Device[]>('get_devices')
    await animateList()
  } catch {
    // 静默：空列表即可
  }
}

async function scan() {
  if (scanning.value) return
  scanning.value = true
  try {
    const found = await invoke<Device[]>('scan_devices')
    mergeDevices(found)
    await animateList()
    toast({ content: `发现 ${found.length} 台设备`, type: 'success' })
  } catch (e) {
    toast({ content: `扫描失败：${e}`, type: 'error' })
  } finally {
    scanning.value = false
  }
}

async function pair(d: Device) {
  if (pairingId.value) return
  pairingId.value = d.id
  const prev = d.status
  d.status = 'pairing' // 乐观更新
  try {
    await invoke('pair_device', { id: d.id })
    d.status = 'paired'
    toast({ content: `已与「${d.name}」配对`, type: 'success' })
  } catch (e) {
    d.status = prev
    toast({ content: `配对失败：${e}`, type: 'error' })
  } finally {
    pairingId.value = null
  }
}

// 状态徽标配置：根据状态返回展示文本
const statusLabels: Record<DeviceStatus, string> = {
  discovered: '已发现',
  paired: '已配对',
  offline: '离线',
  pairing: '配对中',
}

function statusLabel(s: DeviceStatus): string {
  return statusLabels[s] ?? s
}

// 派生：每个设备的徽标 class
function statusClass(s: DeviceStatus): string {
  return `device-badge--${s}`
}

onMounted(async () => {
  await loadDevices()

  unlistenFound = await listen<DeviceFoundPayload>('device-found', (e) => {
    const d = e.payload.device
    const idx = devices.value.findIndex((x) => x.id === d.id)
    if (idx >= 0) {
      devices.value[idx] = d
    } else {
      devices.value.push(d)
    }
  })

  unlistenStatus = await listen<DeviceStatusChangedPayload>(
    'device-status-changed',
    (e) => {
      const { device_id, status } = e.payload
      const d = devices.value.find((x) => x.id === device_id)
      if (d) d.status = status
    },
  )
})

onUnmounted(() => {
  unlistenFound?.()
  unlistenStatus?.()
})
</script>

<template>
  <div class="device-panel">
    <div class="device-head">
      <h2 class="device-title">设备</h2>
      <Button
        variant="primary"
        size="sm"
        :loading="scanning"
        @click="scan"
      >
        {{ scanning ? '扫描中…' : '扫描设备' }}
      </Button>
    </div>

    <div class="device-list">
      <div v-if="devices.length === 0" class="empty-hint">
        暂无设备，点击"扫描设备"开始发现。
      </div>
      <div v-for="d in devices" :key="d.id" class="device-item">
        <div class="device-top">
          <span class="device-name" :title="d.name">{{ d.name }}</span>
          <span class="device-badge" :class="statusClass(d.status)">
            {{ statusLabel(d.status) }}
          </span>
        </div>
        <div class="device-addr">{{ d.address }}</div>
        <div class="device-bottom">
          <span class="device-lastseen">last_seen {{ d.last_seen }}</span>
          <Button
            :variant="d.status === 'paired' ? 'normal' : 'primary'"
            size="sm"
            :disabled="d.status !== 'discovered' || pairingId === d.id"
            :loading="pairingId === d.id"
            @click="pair(d)"
          >
            {{ d.status === 'paired' ? '已配对' : pairingId === d.id ? '配对中' : '配对' }}
          </Button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.device-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
}

.device-item {
  padding: 12px 14px;
  background: var(--card);
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
  gap: 10px;
}

.device-name {
  font-weight: 600;
  font-size: 14px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.device-addr {
  font-size: 12px;
  color: var(--muted);
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.device-lastseen {
  font-size: 11px;
  color: var(--muted);
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.device-bottom {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

/* 状态徽标：用 design tokens 统一颜色 */
.device-badge {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: var(--radius-full);
  white-space: nowrap;
  border: 1px solid transparent;
}

.device-badge--discovered {
  color: var(--muted);
  background: var(--card-2);
  border-color: var(--border);
}

.device-badge--paired {
  color: var(--success);
  background: rgba(62, 207, 142, 0.12);
  border-color: rgba(62, 207, 142, 0.4);
}

.device-badge--offline {
  color: var(--warn);
  background: rgba(240, 192, 74, 0.12);
  border-color: rgba(240, 192, 74, 0.4);
}

.device-badge--pairing {
  color: var(--primary);
  background: rgba(74, 126, 255, 0.12);
  border-color: rgba(74, 126, 255, 0.4);
}
</style>
