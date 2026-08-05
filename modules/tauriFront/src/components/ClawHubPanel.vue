<script setup lang="ts">
/**
 * ClawHubPanel 浏览 / 安装面板
 *
 * 功能：
 * - 双标签：技能（Skills） / 插件（Plugins）
 * - 搜索框（300ms 防抖），调用 clawhub_search_skills / clawhub_search_plugins
 * - 列表浏览，每项显示名称、所有者、版本、简介与安装按钮
 * - 已安装项目以本地状态高亮，支持卸载
 * - 列表底部「加载更多」分页（next_cursor）
 *
 * 容器复用 BindSheet side="right"，与 SkillPanel 一致。
 */
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { BindSheet, Button, SegmentedButton, IconButton, Icon, useToast, type SegmentedOption } from './basic'
import type {
  Skill,
  SkillListItem,
  SkillListResponse,
  SearchResponse,
  ClawHubSearchResult,
  PackageCatalogItem,
  PackageListResponse,
  PackageSearchResponse,
  PackageSearchResult,
  InstalledPlugin,
} from '../types'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

const { toast } = useToast()

// ---------- 标签切换 ----------
type Tab = 'skills' | 'plugins'
const tab = ref<string | number>('skills')

const tabOptions: SegmentedOption[] = [
  { label: '技能', value: 'skills' },
  { label: '插件', value: 'plugins' },
]

// ---------- 数据 ----------
const skillItems = ref<SkillListItem[]>([])
const skillCursor = ref<string | null>(null)
const pluginItems = ref<PackageCatalogItem[]>([])
const pluginCursor = ref<string | null>(null)
const loading = ref(false)
const loadingMore = ref(false)
const installing = ref<Set<string>>(new Set())

// 搜索
const searchQuery = ref('')
let searchTimer: number | null = null

// 已安装状态（本地）
const installedSkills = ref<Skill[]>([])
const installedPlugins = ref<InstalledPlugin[]>([])

// 搜索结果（与列表项分开存储，便于切换显示）
const skillSearchResults = ref<ClawHubSearchResult[]>([])
const pluginSearchResults = ref<PackageSearchResult[]>([])

const isSearching = computed(() => searchQuery.value.trim().length > 0)

// ---------- 加载 ----------
async function refreshInstalled() {
  try {
    const [skills, plugins] = await Promise.all([
      invoke<Skill[]>('list_skills'),
      invoke<InstalledPlugin[]>('list_installed_plugins'),
    ])
    installedSkills.value = skills
    installedPlugins.value = plugins
  } catch (e) {
    console.warn('refreshInstalled failed', e)
  }
}

async function loadSkills(reset = true) {
  if (reset) {
    skillItems.value = []
    skillCursor.value = null
  }
  loading.value = reset
  loadingMore.value = !reset
  try {
    const resp = await invoke<SkillListResponse>('clawhub_list_skills', {
      limit: 20,
      cursor: reset ? null : skillCursor.value,
    })
    skillItems.value.push(...resp.items)
    skillCursor.value = resp.next_cursor ?? null
  } catch (e) {
    toast({ content: `加载 ClawHub 技能失败：${e}`, type: 'error' })
  } finally {
    loading.value = false
    loadingMore.value = false
  }
}

async function loadPlugins(reset = true) {
  if (reset) {
    pluginItems.value = []
    pluginCursor.value = null
  }
  loading.value = reset
  loadingMore.value = !reset
  try {
    const resp = await invoke<PackageListResponse>('clawhub_list_plugins', {
      limit: 20,
      cursor: reset ? null : pluginCursor.value,
    })
    pluginItems.value.push(...resp.items)
    pluginCursor.value = resp.next_cursor ?? null
  } catch (e) {
    toast({ content: `加载 ClawHub 插件失败：${e}`, type: 'error' })
  } finally {
    loading.value = false
    loadingMore.value = false
  }
}

async function refresh() {
  await refreshInstalled()
  if (tab.value === 'skills') {
    await loadSkills(true)
  } else {
    await loadPlugins(true)
  }
}

onMounted(() => {
  refresh()
})

// ---------- 监听技能安装/卸载事件 ----------
// 后端 emit 时机：
// - clawhub-skill-installed：用户在面板点安装 / agent 调用 install_clawhub_skill 工具成功
// - clawhub-skill-uninstalled：用户在面板点卸载
// 收到事件后只需重新拉取 list_skills 同步已安装状态，无需重载 ClawHub 远程列表
let unlistens: UnlistenFn[] = []

async function setupListeners() {
  unlistens.push(
    await listen('clawhub-skill-installed', () => {
      // 不阻塞事件流，仅刷新已安装状态
      refreshInstalled().catch((e) => console.warn('clawhub-skill-installed refresh failed', e))
    }),
  )
  unlistens.push(
    await listen('clawhub-skill-uninstalled', () => {
      refreshInstalled().catch((e) => console.warn('clawhub-skill-uninstalled refresh failed', e))
    }),
  )
}

onMounted(() => {
  setupListeners()
})

onUnmounted(() => {
  unlistens.forEach((fn) => fn?.())
  unlistens = []
})

watch(
  () => props.open,
  (v) => {
    if (v) refresh()
  },
)

watch(tab, () => {
  // 切换标签时清空搜索
  searchQuery.value = ''
  skillSearchResults.value = []
  pluginSearchResults.value = []
  if (tab.value === 'skills' && skillItems.value.length === 0) {
    loadSkills(true)
  } else if (tab.value === 'plugins' && pluginItems.value.length === 0) {
    loadPlugins(true)
  }
})

// ---------- 搜索 ----------
watch(searchQuery, (q) => {
  if (searchTimer) {
    clearTimeout(searchTimer)
    searchTimer = null
  }
  const trimmed = q.trim()
  if (!trimmed) {
    skillSearchResults.value = []
    pluginSearchResults.value = []
    return
  }
  searchTimer = window.setTimeout(async () => {
    loading.value = true
    try {
      if (tab.value === 'skills') {
        const resp = await invoke<SearchResponse>('clawhub_search_skills', { q: trimmed, limit: 30 })
        skillSearchResults.value = resp.results
      } else {
        const resp = await invoke<PackageSearchResponse>('clawhub_search_plugins', { q: trimmed, limit: 30 })
        pluginSearchResults.value = resp.results
      }
    } catch (e) {
      toast({ content: `搜索失败：${e}`, type: 'error' })
    } finally {
      loading.value = false
    }
  }, 300)
})

// ---------- 安装 / 卸载 ----------
function isSkillInstalled(slug: string): boolean {
  return installedSkills.value.some((s) => s.source_slug === slug)
}

function isPluginInstalled(name: string): boolean {
  return installedPlugins.value.some((p) => p.name === name)
}

async function installSkill(slug: string) {
  installing.value.add(slug)
  try {
    await invoke('clawhub_install_skill', { slug })
    await refreshInstalled()
    toast({ content: `技能 ${slug} 安装成功`, type: 'success' })
  } catch (e) {
    toast({ content: `安装失败：${e}`, type: 'error' })
  } finally {
    installing.value.delete(slug)
  }
}

async function uninstallSkill(id: string) {
  try {
    await invoke('clawhub_uninstall_skill', { id })
    await refreshInstalled()
    toast({ content: `已卸载 ${id}`, type: 'success' })
  } catch (e) {
    toast({ content: `卸载失败：${e}`, type: 'error' })
  }
}

async function installPlugin(name: string) {
  installing.value.add(name)
  try {
    await invoke('clawhub_install_plugin', { name })
    await refreshInstalled()
    toast({ content: `插件 ${name} 安装成功`, type: 'success' })
  } catch (e) {
    toast({ content: `安装失败：${e}`, type: 'error' })
  } finally {
    installing.value.delete(name)
  }
}

async function uninstallPlugin(id: string) {
  try {
    await invoke('clawhub_uninstall_plugin', { id })
    await refreshInstalled()
    toast({ content: `已卸载 ${id}`, type: 'success' })
  } catch (e) {
    toast({ content: `卸载失败：${e}`, type: 'error' })
  }
}

// ---------- 渲染辅助 ----------
function formatTime(ts?: number | null): string {
  if (!ts) return ''
  const diff = Date.now() - ts
  if (diff < 86400000) return '今日'
  if (diff < 2592000000) return `${Math.floor(diff / 86400000)} 天前`
  try {
    return new Date(ts).toLocaleDateString()
  } catch {
    return ''
  }
}

// 安装中标记
function isInstalling(key: string): boolean {
  return installing.value.has(key)
}

// 加载更多
const canLoadMore = computed(() => {
  if (tab.value === 'skills') return skillCursor.value !== null
  return pluginCursor.value !== null
})

async function loadMore() {
  if (tab.value === 'skills') {
    await loadSkills(false)
  } else {
    await loadPlugins(false)
  }
}
</script>

<template>
  <BindSheet
    :visible="props.open"
    side="right"
    width="540px"
    title="ClawHub 商店"
    @update:visible="(v) => !v && emit('close')"
  >
    <div class="clawhub-body">
      <!-- 标签切换 -->
      <div class="clawhub-tabs">
        <SegmentedButton
          v-model="tab"
          :options="tabOptions"
        />
      </div>

      <!-- 搜索框 -->
      <div class="clawhub-search">
        <span class="search-icon"><Icon name="search" :size="18" /></span>
        <input
          v-model="searchQuery"
          type="text"
          class="search-input"
          :placeholder="tab === 'skills' ? '搜索 ClawHub 技能...' : '搜索 ClawHub 插件...'"
        />
        <IconButton
          v-if="searchQuery"
          size="sm"
          title="清空"
          @click="searchQuery = ''"
        ><Icon name="close" :size="16" /></IconButton>
      </div>

      <!-- 内容区 -->
      <div class="clawhub-list">
        <!-- 加载中 -->
        <div v-if="loading" class="empty-hint">
          <Icon name="loader" :size="24" /> 加载中...
        </div>

        <!-- 技能：搜索结果 -->
        <template v-else-if="tab === 'skills' && isSearching">
          <div v-if="skillSearchResults.length === 0" class="empty-hint">
            未找到相关技能
          </div>
          <div
            v-for="r in skillSearchResults"
            :key="r.slug || r.display_name || 'unknown'"
            class="card"
          >
            <div class="card-main">
              <div class="card-title">
                {{ r.display_name || r.slug || '未命名' }}
                <span v-if="r.version" class="card-version">v{{ r.version }}</span>
              </div>
              <div class="card-meta">
                <span v-if="r.owner_handle" class="meta-owner">@{{ r.owner_handle }}</span>
                <span v-if="r.updated_at" class="meta-time">{{ formatTime(r.updated_at) }}</span>
              </div>
              <div v-if="r.summary" class="card-summary">{{ r.summary }}</div>
            </div>
            <div class="card-actions">
              <Button
                v-if="r.slug && !isSkillInstalled(r.slug)"
                size="sm"
                variant="primary"
                :loading="isInstalling(r.slug)"
                :disabled="!r.slug"
                @click="r.slug && installSkill(r.slug)"
              >
                安装
              </Button>
              <Button
                v-else-if="r.slug && isSkillInstalled(r.slug)"
                size="sm"
                variant="text"
                @click="r.slug && uninstallSkill(r.slug)"
              >
                卸载
              </Button>
              <span v-else class="meta-muted">不可安装</span>
            </div>
          </div>
        </template>

        <!-- 技能：列表 -->
        <template v-else-if="tab === 'skills'">
          <div v-if="skillItems.length === 0" class="empty-hint">
            暂无技能
          </div>
          <div
            v-for="item in skillItems"
            :key="item.slug"
            class="card"
          >
            <div class="card-main">
              <div class="card-title">
                {{ item.display_name }}
                <span v-if="item.latest_version" class="card-version">v{{ item.latest_version.version }}</span>
                <span v-if="isSkillInstalled(item.slug)" class="card-badge">已安装</span>
              </div>
              <div class="card-meta">
                <span v-if="item.topics && item.topics.length" class="meta-topic">{{ item.topics[0] }}</span>
                <span v-if="item.updated_at" class="meta-time">{{ formatTime(item.updated_at) }}</span>
              </div>
              <div v-if="item.summary" class="card-summary">{{ item.summary }}</div>
            </div>
            <div class="card-actions">
              <Button
                v-if="!isSkillInstalled(item.slug)"
                size="sm"
                variant="primary"
                :loading="isInstalling(item.slug)"
                @click="installSkill(item.slug)"
              >
                安装
              </Button>
              <Button
                v-else
                size="sm"
                variant="text"
                @click="uninstallSkill(item.slug)"
              >
                卸载
              </Button>
            </div>
          </div>
        </template>

        <!-- 插件：搜索结果 -->
        <template v-else-if="tab === 'plugins' && isSearching">
          <div v-if="pluginSearchResults.length === 0" class="empty-hint">
            未找到相关插件
          </div>
          <div
            v-for="r in pluginSearchResults"
            :key="r.name || r.display_name || ''"
            class="card"
          >
            <div class="card-main">
              <div class="card-title">
                {{ r.display_name || r.name || '未命名' }}
              </div>
              <div class="card-meta">
                <span v-if="r.owner_handle" class="meta-owner">@{{ r.owner_handle }}</span>
                <span v-if="r.family" class="meta-family">{{ r.family }}</span>
              </div>
              <div v-if="r.summary" class="card-summary">{{ r.summary }}</div>
            </div>
            <div class="card-actions">
              <Button
                v-if="r.name && !isPluginInstalled(r.name)"
                size="sm"
                variant="primary"
                :loading="isInstalling(r.name)"
                :disabled="!r.name"
                @click="r.name && installPlugin(r.name)"
              >
                安装
              </Button>
              <Button
                v-else-if="r.name && isPluginInstalled(r.name)"
                size="sm"
                variant="text"
                @click="r.name && uninstallPlugin(r.name)"
              >
                卸载
              </Button>
            </div>
          </div>
        </template>

        <!-- 插件：列表 -->
        <template v-else>
          <div v-if="pluginItems.length === 0" class="empty-hint">
            暂无插件
          </div>
          <div
            v-for="item in pluginItems"
            :key="item.name"
            class="card"
          >
            <div class="card-main">
              <div class="card-title">
                {{ item.display_name }}
                <span v-if="item.latest_version" class="card-version">v{{ item.latest_version }}</span>
                <span v-if="item.is_official" class="card-official">官方</span>
                <span v-if="isPluginInstalled(item.name)" class="card-badge">已安装</span>
              </div>
              <div class="card-meta">
                <span v-if="item.owner_handle" class="meta-owner">@{{ item.owner_handle }}</span>
                <span v-if="item.family" class="meta-family">{{ item.family }}</span>
                <span v-if="item.channel" class="meta-channel">{{ item.channel }}</span>
              </div>
              <div v-if="item.summary" class="card-summary">{{ item.summary }}</div>
            </div>
            <div class="card-actions">
              <Button
                v-if="!isPluginInstalled(item.name)"
                size="sm"
                variant="primary"
                :loading="isInstalling(item.name)"
                @click="installPlugin(item.name)"
              >
                安装
              </Button>
              <Button
                v-else
                size="sm"
                variant="text"
                @click="uninstallPlugin(item.name)"
              >
                卸载
              </Button>
            </div>
          </div>
        </template>

        <!-- 加载更多 -->
        <div v-if="canLoadMore && !isSearching && !loading" class="load-more">
          <Button
            variant="text"
            block
            :loading="loadingMore"
            @click="loadMore"
          >
            加载更多
          </Button>
        </div>
      </div>

      <!-- 底部：已安装入口提示 -->
      <div class="clawhub-footer">
        <span class="footer-hint">
          已安装 {{ installedSkills.filter(s => s.source === 'clawhub').length }} 个技能 / {{ installedPlugins.length }} 个插件
        </span>
        <Button variant="text" size="sm" @click="refresh">
          <template #icon><Icon name="refresh" :size="16" /></template>
          刷新
        </Button>
      </div>
    </div>
  </BindSheet>
</template>

<style scoped>
.clawhub-body {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

.clawhub-tabs {
  padding: 8px 16px 0;
  flex-shrink: 0;
}

.clawhub-search {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 12px 16px 8px;
  padding: 0 12px;
  height: var(--h-control-md);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  flex-shrink: 0;
}

.search-icon {
  font-size: 14px;
  color: var(--muted);
}

.search-input {
  flex: 1;
  border: none;
  background: transparent;
  color: var(--text);
  font-size: 14px;
  outline: none;
}

.clawhub-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px 16px;
}

.empty-hint {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 48px 16px;
  color: var(--muted);
  font-size: 14px;
}

.card {
  display: flex;
  gap: 12px;
  padding: 12px 14px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  margin-bottom: 8px;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.card:hover {
  border-color: var(--border-hover, var(--border));
}

.card-main {
  flex: 1;
  min-width: 0;
}

.card-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.card-version {
  font-size: 11px;
  font-weight: 400;
  color: var(--muted);
  padding: 1px 6px;
  background: var(--card-2);
  border-radius: var(--radius-full);
}

.card-badge {
  font-size: 11px;
  font-weight: 500;
  color: var(--success, #10a37f);
  padding: 1px 6px;
  background: rgba(16, 163, 127, 0.1);
  border-radius: var(--radius-full);
}

.card-official {
  font-size: 11px;
  font-weight: 500;
  color: var(--primary, #4a7eff);
  padding: 1px 6px;
  background: rgba(74, 126, 255, 0.1);
  border-radius: var(--radius-full);
}

.card-meta {
  display: flex;
  gap: 8px;
  margin-top: 4px;
  font-size: 12px;
  color: var(--muted);
  flex-wrap: wrap;
}

.meta-owner {
  color: var(--primary, #4a7eff);
}

.meta-family,
.meta-channel,
.meta-topic {
  padding: 0 6px;
  background: var(--card-2);
  border-radius: var(--radius-md);
}

.meta-time {
  color: var(--muted);
}

.meta-muted {
  font-size: 12px;
  color: var(--muted);
}

.card-summary {
  margin-top: 6px;
  font-size: 12px;
  color: var(--text);
  opacity: 0.75;
  line-height: 1.4;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.card-actions {
  flex-shrink: 0;
  display: flex;
  align-items: center;
}

.load-more {
  padding: 8px 0 16px;
}

.clawhub-footer {
  padding: 12px 16px;
  border-top: 1px solid var(--border);
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-shrink: 0;
}

.footer-hint {
  font-size: 12px;
  color: var(--muted);
}
</style>
