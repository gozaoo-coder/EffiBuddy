/**
 * useHistorySelection —— 历史记录多选状态管理 composable
 *
 * 从 HistoryRail.vue 抽取的多选逻辑：
 * - selectionMode：是否处于多选模式
 * - selectedIds：已选中的会话 id 集合（Set，O(1) 查找）
 * - 切换选中 / 全选 / 取消全选 / 反选 / 退出
 *
 * 设计要点：
 * - 使用 Set<string> 而非数组，查找/删除 O(1)
 * - 每次变更都创建新 Set（而非 mutate），确保 Vue 响应式触发
 * - 退出多选模式时自动清空选中集合
 * - 不关心列表数据本身，只管选中状态；列表由 HistoryRail 提供
 */
import { ref, computed } from 'vue'

export function useHistorySelection() {
  /** 是否处于多选模式 */
  const selectionMode = ref(false)
  /** 已选中的会话 id 集合 */
  const selectedIds = ref<Set<string>>(new Set())

  const selectedCount = computed(() => selectedIds.value.size)

  /** 进入多选模式 */
  function enterSelectionMode() {
    selectionMode.value = true
    selectedIds.value = new Set()
  }

  /** 退出多选模式（清空选中） */
  function exitSelectionMode() {
    selectionMode.value = false
    selectedIds.value = new Set()
  }

  /** 切换某条会话的选中状态 */
  function toggleSelected(id: string) {
    const next = new Set(selectedIds.value)
    if (next.has(id)) {
      next.delete(id)
    } else {
      next.add(id)
    }
    selectedIds.value = next
  }

  /** 是否选中 */
  function isSelected(id: string): boolean {
    return selectedIds.value.has(id)
  }

  /** 全选（基于传入的 id 列表） */
  function selectAll(ids: string[]) {
    const next = new Set(selectedIds.value)
    for (const id of ids) {
      next.add(id)
    }
    selectedIds.value = next
  }

  /** 取消全选（清空） */
  function selectNone() {
    selectedIds.value = new Set()
  }

  /** 反选（基于传入的完整 id 列表） */
  function selectInverse(ids: string[]) {
    const next = new Set<string>()
    for (const id of ids) {
      if (!selectedIds.value.has(id)) {
        next.add(id)
      }
    }
    selectedIds.value = next
  }

  /** 获取已选 id 数组 */
  function getSelectedArray(): string[] {
    return Array.from(selectedIds.value)
  }

  return {
    selectionMode,
    selectedIds,
    selectedCount,
    enterSelectionMode,
    exitSelectionMode,
    toggleSelected,
    isSelected,
    selectAll,
    selectNone,
    selectInverse,
    getSelectedArray,
  }
}
