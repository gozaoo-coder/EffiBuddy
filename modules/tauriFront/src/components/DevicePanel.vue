<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { animate, stagger } from 'animejs'
import type { Device, DeviceStatus, DeviceFoundPayload, DeviceStatusChangedPayload } from '../types'

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
// anime.js v4 在目标选择器匹配不到任何元素时会抛出
// "No target found"，因此需在动画前检查 DOM 是否就绪。
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
  } catch {
    // 忽略扫描错误，保持现有列表
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
  } catch {
    d.status = prev
  } finally {
    pairingId.value = null
  }
}

function statusLabel(s: DeviceStatus): string {
  return s
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
      <h2>设备</h2>
      <button class="scan-btn" :disabled="scanning" @click="scan">
        {{ scanning ? '扫描中…' : '扫描设备' }}
      </button>
    </div>

    <div class="device-list">
      <div v-if="devices.length === 0" class="empty-hint">
        暂无设备，点击"扫描设备"开始发现。
      </div>
      <div v-for="d in devices" :key="d.id" class="device-item">
        <div class="device-top">
          <span class="device-name" :title="d.name">{{ d.name }}</span>
          <span class="badge" :class="d.status">{{ statusLabel(d.status) }}</span>
        </div>
        <div class="device-addr">{{ d.address }}</div>
        <div class="device-bottom">
          <span class="device-addr">last_seen {{ d.last_seen }}</span>
          <button
            class="pair-btn"
            :disabled="d.status !== 'discovered' || pairingId === d.id"
            @click="pair(d)"
          >
            {{ d.status === 'paired' ? '已配对' : pairingId === d.id ? '配对中…' : '配对' }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
