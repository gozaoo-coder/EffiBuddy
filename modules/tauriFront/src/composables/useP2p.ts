/**
 * useP2p — P2P 设备发现、配对、同步的状态管理与 Tauri 命令封装。
 *
 * # 职责
 * - 持有 P2P 全局状态：服务状态、设备列表、待处理配对请求
 * - 封装所有 P2P Tauri 命令调用（invoke wrapper），统一错误处理
 * - 监听 Tauri 事件（device-found / device-status-changed / pairing-request），实时更新状态
 * - 提供配对/取消配对/同步等操作的语义化方法，供组件直接调用
 *
 * # 设计
 * - 单例模式：模块级 `p2pState` 被 `useP2p()` 共享，多组件实例看到同一份状态
 * - 事件监听在首次 `useP2p()` 调用时安装，组件卸载不卸载（全局生命周期）
 * - 状态用 `ref` 持有，Vue 响应式自动驱动 UI 更新
 * - 所有 async 方法返回 `Promise<void>`，错误经 toast 反馈，异常不向上抛
 */
import { ref, computed, type Ref, type ComputedRef } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type {
  Device,
  DeviceStatus,
  PairingRequest,
  PairRole,
  P2pStatus,
  DeviceFoundPayload,
  DeviceStatusChangedPayload,
  PairingRequestPayload,
} from '../types'

// ── 全局单例状态 ────────────────────────────────────────────────────────

/** P2P 服务状态（started / self_device_id） */
const p2pStatus = ref<P2pStatus>({ started: false, self_device_id: '' })

/** 已知设备列表（已配对 + 已发现 + 在线状态合并） */
const devices = ref<Device[]>([])

/** 待处理配对请求（广播发现 → 对端请求 → 本机准许） */
const pendingRequests = ref<PairingRequest[]>([])

/** 操作进行中标志（防止重复点击） */
const loading = ref(false)

// ── 事件监听安装（幂等，仅首次安装） ────────────────────────────────────

let listenersInstalled = false
let unlistenFound: UnlistenFn | null = null
let unlistenStatus: UnlistenFn | null = null
let unlistenPairingReq: UnlistenFn | null = null

/** 合并设备：按 id 去重，新数据覆盖旧数据 */
function mergeDevices(incoming: Device[]): void {
  const map = new Map<string, Device>()
  for (const d of devices.value) map.set(d.id, d)
  for (const d of incoming) map.set(d.id, d)
  devices.value = [...map.values()]
}

/** 安装全局事件监听（幂等） */
async function installListeners(): Promise<void> {
  if (listenersInstalled) return
  listenersInstalled = true

  unlistenFound = await listen<DeviceFoundPayload>('device-found', (e) => {
    const d = e.payload.device
    mergeDevices([d])
  })

  unlistenStatus = await listen<DeviceStatusChangedPayload>(
    'device-status-changed',
    (e) => {
      const { device_id, status } = e.payload
      const d = devices.value.find((x) => x.id === device_id)
      if (d) d.status = status
    },
  )

  // 配对请求：主机跳出可点击的 msg bubble 提醒
  unlistenPairingReq = await listen<PairingRequestPayload>(
    'pairing-request',
    (e) => {
      const d = e.payload.device
      // 加入待处理列表（去重）
      if (!pendingRequests.value.some((r) => r.device_id === d.id)) {
        pendingRequests.value.push({
          device_id: d.id,
          name: d.name,
          address: d.address,
          pubkey_hex: '',
          timestamp: d.last_seen,
        })
      }
      // 同时更新设备列表
      mergeDevices([d])
    },
  )
}

// ── Tauri 命令封装 ──────────────────────────────────────────────────────

/** 查询 P2P 服务状态 */
async function fetchStatus(): Promise<void> {
  try {
    p2pStatus.value = await invoke<P2pStatus>('get_p2p_status')
  } catch {
    // 静默：状态保持默认
  }
}

/** 拉取已知设备列表 */
async function fetchDevices(): Promise<void> {
  try {
    const list = await invoke<Device[]>('get_devices')
    mergeDevices(list)
  } catch {
    // 静默
  }
}

/** 拉取待处理配对请求列表 */
async function fetchPendingRequests(): Promise<void> {
  try {
    pendingRequests.value = await invoke<PairingRequest[]>('pending_pairing_requests')
  } catch {
    // 静默
  }
}

/** 刷新全部状态（面板打开时调用） */
async function refreshAll(): Promise<void> {
  await Promise.all([fetchStatus(), fetchDevices(), fetchPendingRequests()])
}

/** 触发一次 UDP 广播扫描 */
async function scan(): Promise<Device[]> {
  loading.value = true
  try {
    const found = await invoke<Device[]>('scan_devices')
    mergeDevices(found)
    return found
  } finally {
    loading.value = false
  }
}

/** 方法一：通过 IP/链接直连配对（首次配对交换可信密钥） */
async function pairByAddress(address: string, role: PairRole = 'mirror'): Promise<Device | null> {
  loading.value = true
  try {
    const device = await invoke<Device>('pair_by_address', { address, role })
    mergeDevices([device])
    return device
  } finally {
    loading.value = false
  }
}

/** 方法二：接受已发现设备的配对请求 */
async function acceptPair(deviceId: string, role: PairRole = 'mirror'): Promise<boolean> {
  loading.value = true
  try {
    await invoke('pair_device', { id: deviceId, role })
    // 从待处理列表移除
    pendingRequests.value = pendingRequests.value.filter((r) => r.device_id !== deviceId)
    return true
  } catch {
    return false
  } finally {
    loading.value = false
  }
}

/** 拒绝配对请求 */
async function rejectPair(deviceId: string): Promise<void> {
  try {
    await invoke('reject_pair', { deviceId })
    pendingRequests.value = pendingRequests.value.filter((r) => r.device_id !== deviceId)
  } catch {
    // 静默
  }
}

/** 取消已配对设备 */
async function unpair(deviceId: string): Promise<boolean> {
  loading.value = true
  try {
    await invoke('unpair', { deviceId })
    const d = devices.value.find((x) => x.id === deviceId)
    if (d) d.status = 'offline' as DeviceStatus
    return true
  } catch {
    return false
  } finally {
    loading.value = false
  }
}

/** 镜像同步：从指定设备拉取数据 */
async function syncPull(
  deviceId: string,
  since: number = 0,
  kinds: string[] = [],
): Promise<boolean> {
  loading.value = true
  try {
    await invoke('sync_pull', { deviceId, since, kinds })
    return true
  } catch {
    return false
  } finally {
    loading.value = false
  }
}

/** 镜像同步：向指定设备推送数据 */
async function syncPush(deviceId: string, kinds: string[] = []): Promise<boolean> {
  loading.value = true
  try {
    await invoke('sync_push', { deviceId, kinds })
    return true
  } catch {
    return false
  } finally {
    loading.value = false
  }
}

/** 查询与指定设备的同步进度 */
async function syncCursor(deviceId: string): Promise<number> {
  try {
    return await invoke<number>('sync_cursor', { deviceId })
  } catch {
    return 0
  }
}

/** 启动后台持续发现 */
async function startDiscovery(): Promise<boolean> {
  try {
    await invoke('start_discovery')
    return true
  } catch {
    return false
  }
}

/** 停止后台持续发现 */
async function stopDiscovery(): Promise<boolean> {
  try {
    await invoke('stop_discovery')
    return true
  } catch {
    return false
  }
}

// ── composable 入口 ─────────────────────────────────────────────────────

export interface UseP2p {
  // 响应式状态
  status: Ref<P2pStatus>
  devices: Ref<Device[]>
  pendingRequests: Ref<PairingRequest[]>
  loading: Ref<boolean>
  // 派生
  onlineDevices: ComputedRef<Device[]>
  pendingCount: ComputedRef<number>
  // 方法
  refreshAll: () => Promise<void>
  scan: () => Promise<Device[]>
  pairByAddress: (address: string, role?: PairRole) => Promise<Device | null>
  acceptPair: (deviceId: string, role?: PairRole) => Promise<boolean>
  rejectPair: (deviceId: string) => Promise<void>
  unpair: (deviceId: string) => Promise<boolean>
  syncPull: (deviceId: string, since?: number, kinds?: string[]) => Promise<boolean>
  syncPush: (deviceId: string, kinds?: string[]) => Promise<boolean>
  syncCursor: (deviceId: string) => Promise<number>
  startDiscovery: () => Promise<boolean>
  stopDiscovery: () => Promise<boolean>
}

/**
 * P2P 状态管理 composable（单例）。
 *
 * 首次调用安装全局事件监听并刷新状态；后续调用共享同一份状态。
 */
export function useP2p(): UseP2p {
  // 安装事件监听（异步，不阻塞返回）
  installListeners()

  // 派生状态：computed 自动追踪依赖，状态变化时重新计算
  const onlineDevices = computed(() => devices.value.filter((d) => d.status === 'paired'))
  const pendingCount = computed(() => pendingRequests.value.length)

  return {
    status: p2pStatus,
    devices,
    pendingRequests,
    loading,
    onlineDevices,
    pendingCount,
    refreshAll,
    scan,
    pairByAddress,
    acceptPair,
    rejectPair,
    unpair,
    syncPull,
    syncPush,
    syncCursor,
    startDiscovery,
    stopDiscovery,
  }
}
