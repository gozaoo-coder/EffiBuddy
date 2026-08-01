/**
 * useConversationFolders —— 会话文件夹管理 composable
 *
 * 从 HistoryRail.vue 抽取的文件夹状态与操作逻辑：
 * - 文件夹列表与「会话→文件夹」映射持久化到 localStorage
 * - 文件夹的增删改名
 * - 会话在文件夹间的移动
 * - 文件夹筛选状态（null=全部 / 'unclassified'=未分类 / folder_id=指定文件夹）
 *
 * 设计要点：
 * - `activeFolderId` 支持三态：null / 'unclassified' / folder_id
 *   'unclassified' 是前端虚拟文件夹，筛选未归入任何文件夹的会话
 * - `saveFolders` 在每次变更后自动持久化，调用方无需手动调
 * - 不持有会话列表本身（由 HistoryRail 管理），只管文件夹映射
 */
import { ref } from 'vue'

export interface ConvFolder {
  id: string
  name: string
  created_at: number
}

/** activeFolderId 的特殊值：未分类 */
export const UNCLASSIFIED = 'unclassified'

const FOLDERS_KEY = 'effisuite_conv_folders'
const FOLDER_MAP_KEY = 'effisuite_conv_folder_map'

export function useConversationFolders() {
  const folders = ref<ConvFolder[]>([])
  /** conversation_id → folder_id */
  const convFolderMap = ref<Record<string, string>>({})
  /** 当前筛选的文件夹 id（null = 全部 / UNCLASSIFIED = 未分类 / folder_id = 指定） */
  const activeFolderId = ref<string | null>(null)

  function loadFolders() {
    try {
      folders.value = JSON.parse(localStorage.getItem(FOLDERS_KEY) || '[]')
      convFolderMap.value = JSON.parse(localStorage.getItem(FOLDER_MAP_KEY) || '{}')
    } catch {
      folders.value = []
      convFolderMap.value = {}
    }
  }

  function saveFolders() {
    localStorage.setItem(FOLDERS_KEY, JSON.stringify(folders.value))
    localStorage.setItem(FOLDER_MAP_KEY, JSON.stringify(convFolderMap.value))
  }

  /** 统计指定文件夹中的会话数 */
  function folderConvCount(folderId: string): number {
    return Object.values(convFolderMap.value).filter((id) => id === folderId).length
  }

  /** 统计未分类会话数（需要传入当前所有会话 id 列表） */
  function unclassifiedCount(allConvIds: string[]): number {
    return allConvIds.filter((id) => !convFolderMap.value[id]).length
  }

  /** 获取会话所属的文件夹 id（不存在则为 undefined） */
  function getConvFolder(convId: string): string | undefined {
    return convFolderMap.value[convId]
  }

  /** 创建文件夹 */
  function createFolder(name: string): ConvFolder {
    const folder: ConvFolder = {
      id: `folder_${Date.now()}`,
      name,
      created_at: Date.now(),
    }
    folders.value.push(folder)
    saveFolders()
    return folder
  }

  /** 重命名文件夹 */
  function renameFolder(folder: ConvFolder, name: string) {
    folder.name = name
    saveFolders()
  }

  /** 删除文件夹（同时清除其下所有会话的映射） */
  function removeFolder(folderId: string) {
    folders.value = folders.value.filter((x) => x.id !== folderId)
    for (const k of Object.keys(convFolderMap.value)) {
      if (convFolderMap.value[k] === folderId) delete convFolderMap.value[k]
    }
    if (activeFolderId.value === folderId) activeFolderId.value = null
    saveFolders()
  }

  /** 移动会话到文件夹（folderId 为空则移出文件夹） */
  function moveConvToFolder(convId: string, folderId: string | null) {
    if (folderId) {
      convFolderMap.value[convId] = folderId
    } else {
      delete convFolderMap.value[convId]
    }
    saveFolders()
  }

  /** 批量移动会话到文件夹 */
  function batchMoveConvToFolder(convIds: string[], folderId: string | null) {
    for (const id of convIds) {
      if (folderId) {
        convFolderMap.value[id] = folderId
      } else {
        delete convFolderMap.value[id]
      }
    }
    saveFolders()
  }

  /** 根据文件夹名查找第一个匹配的文件夹（用于自动归类结果匹配） */
  function findFolderByName(name: string): ConvFolder | undefined {
    return folders.value.find((f) => f.name === name)
  }

  /** 所有文件夹名列表（传给后端 auto_classify 用） */
  function folderNames(): string[] {
    return folders.value.map((f) => f.name)
  }

  return {
    folders,
    convFolderMap,
    activeFolderId,
    loadFolders,
    saveFolders,
    folderConvCount,
    unclassifiedCount,
    getConvFolder,
    createFolder,
    renameFolder,
    removeFolder,
    moveConvToFolder,
    batchMoveConvToFolder,
    findFolderByName,
    folderNames,
  }
}
