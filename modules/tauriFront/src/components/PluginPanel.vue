<script setup lang="ts">
/**
 * PluginPanel 已安装插件管理面板
 * 仅展示本地已安装插件，支持卸载（删除）。
 * 容器复用 BindSheet side="right"，风格与 SkillPanel 保持一致。
 */
import { ref, watch, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { BindSheet, Button, Dialog, IconButton, Icon, useToast } from './basic'
import type { InstalledPlugin } from '../types'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{
  (e: 'close'): void
  (e: 'open-clawhub'): void
}>()

const { toast } = useToast()

// ---------- 数据 ----------
const plugins = ref<InstalledPlugin[]>([])
const loading = ref(false)

async function refresh() {
  loading.value = true
  try {
    plugins.value = await invoke<InstalledPlugin[]>('list_installed_plugins')
  } catch (e) {
    toast({ content: `加载插件列表失败：${e}`, type: 'error' })
    plugins.value = []
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  refresh()
})

watch(
  () => props.open,
  (v) => {
    if (v) refresh()
  },
)

// ---------- 删除 ----------
const deleteDialogOpen = ref(false)
const deleteTarget = ref<InstalledPlugin | null>(null)

function startDelete(p: InstalledPlugin) {
  deleteTarget.value = p
  deleteDialogOpen.value = true
}

async function confirmDelete() {
  const p = deleteTarget.value
  if (!p) return
  try {
    await invoke('clawhub_uninstall_plugin', { id: p.id })
    toast({ content: `已删除插件「${p.display_name}」`, type: 'success' })
    await refresh()
  } catch (e) {
    toast({ content: `删除插件失败：${e}`, type: 'error' })
  } finally {
    deleteTarget.value = null
  }
}

function onClose() {
  emit('close')
}
</script>

<template>
  <BindSheet
    :visible="props.open"
    side="right"
    width="520px"
    title="插件管理"
    @close="onClose"
  >
    <div class="plugin-body">
      <!-- 顶部说明 -->
      <header class="plugin-hero">
        <div class="hero-mark"><Icon name="puzzle" :size="28" /></div>
        <div class="hero-text">
          <h2 class="hero-title">已安装插件</h2>
          <p class="hero-sub">管理本地插件，卸载后对应能力将从 agent 中移除</p>
        </div>
      </header>

      <!-- 插件列表 -->
      <section class="section">
        <div class="section-head">
          <span class="section-title">我的插件</span>
          <span v-if="plugins.length" class="count-badge">{{ plugins.length }}</span>
        </div>

        <div v-if="!plugins.length && !loading" class="empty-state">
          <div class="empty-illust"><Icon name="puzzle" :size="48" /></div>
          <p class="empty-text">还没有已安装插件</p>
          <p class="empty-hint">去 ClawHub 商店浏览并安装插件</p>
          <Button variant="text" class="clawhub-entry" @click="emit('open-clawhub')">
            <template #icon><Icon name="globe" :size="18" /></template>
            去 ClawHub 商店浏览
          </Button>
        </div>

        <div v-else class="plugin-list">
          <div
            v-for="p in plugins"
            :key="p.id"
            class="plugin-card"
          >
            <div class="plugin-card-main">
              <span class="plugin-glyph"><Icon name="puzzle" :size="20" /></span>
              <div class="plugin-info">
                <div class="plugin-top">
                  <span class="plugin-name" :title="p.display_name">{{ p.display_name }}</span>
                  <span v-if="p.version" class="version-badge">v{{ p.version }}</span>
                </div>
                <div class="plugin-meta">
                  <span class="meta-id">{{ p.name }}</span>
                  <span v-if="p.family" class="meta-tag">{{ p.family }}</span>
                  <span v-if="p.channel" class="meta-tag">{{ p.channel }}</span>
                  <span v-if="p.owner_handle" class="meta-owner">@{{ p.owner_handle }}</span>
                </div>
                <p v-if="p.summary" class="plugin-summary">{{ p.summary }}</p>
              </div>
            </div>
            <IconButton
              size="sm"
              variant="danger"
              title="删除"
              @click.stop="startDelete(p)"
            >
              <Icon name="delete" :size="18" />
            </IconButton>
          </div>
        </div>
      </section>

      <!-- 底部入口 -->
      <div v-if="plugins.length" class="plugin-footer">
        <Button variant="text" block class="clawhub-entry" @click="emit('open-clawhub')">
          <template #icon><Icon name="globe" :size="18" /></template>
          去 ClawHub 商店浏览
        </Button>
      </div>
    </div>

    <!-- 删除确认 -->
    <Dialog
      v-model:visible="deleteDialogOpen"
      title="删除插件"
      danger
      confirm-text="删除"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmDelete"
    >
      <div class="dialog-delete-content">
        确定删除插件「{{ deleteTarget?.display_name }}」？此操作不可撤销。
      </div>
    </Dialog>
  </BindSheet>
</template>

<style scoped>
.plugin-body {
  padding: 20px 24px 32px;
  display: flex;
  flex-direction: column;
  gap: 20px;
  overflow-y: auto;
}

/* ---------- 顶部说明 ---------- */
.plugin-hero {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 18px 20px;
  background: linear-gradient(135deg, rgba(168, 85, 247, 0.14), rgba(168, 85, 247, 0.02));
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.hero-mark {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  border-radius: var(--radius);
  background: var(--card-2);
  font-size: 22px;
  flex-shrink: 0;
}

.hero-text {
  min-width: 0;
}

.hero-title {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text);
}

.hero-sub {
  margin: 4px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 分区 ---------- */
.section {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.section-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.section-title {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.count-badge {
  padding: 2px 10px;
  font-size: var(--fs-xs);
  color: var(--muted);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
}

/* ---------- 插件卡片 ---------- */
.plugin-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.plugin-card {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--card);
  transition: border-color var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard);
}

.plugin-card:hover {
  border-color: var(--primary);
  background: var(--card-2);
}

.plugin-card-main {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  flex: 1;
  min-width: 0;
}

.plugin-glyph {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-sm);
  background: #a855f7;
  color: #fff;
  font-size: 18px;
  flex-shrink: 0;
}

.plugin-info {
  flex: 1;
  min-width: 0;
}

.plugin-top {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.plugin-name {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.version-badge {
  flex-shrink: 0;
  padding: 1px 8px;
  font-size: var(--fs-xs);
  font-weight: 500;
  color: var(--muted);
  background: var(--card-2);
  border-radius: var(--radius-full);
}

.plugin-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
  font-size: var(--fs-xs);
  color: var(--muted);
  flex-wrap: wrap;
}

.meta-id {
  font-family: var(--font-mono, monospace);
  opacity: 0.8;
}

.meta-tag {
  padding: 1px 6px;
  background: var(--card-2);
  border-radius: var(--radius-sm);
}

.meta-owner {
  color: var(--primary, #4a7eff);
}

.plugin-summary {
  margin: 6px 0 0;
  font-size: var(--fs-sm);
  color: var(--text);
  opacity: 0.75;
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

/* ---------- 空状态 ---------- */
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 36px 20px;
  border: 1px dashed var(--border);
  border-radius: var(--radius);
  background: var(--card);
  text-align: center;
}

.empty-illust {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 56px;
  height: 56px;
  border-radius: 50%;
  background: var(--card-2);
  color: var(--muted);
  font-size: 26px;
  margin-bottom: 4px;
}

.empty-text {
  margin: 0;
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.empty-hint {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.5;
  max-width: 320px;
}

.clawhub-entry {
  margin-top: 4px;
  color: #a855f7;
}

/* ---------- 底部 ---------- */
.plugin-footer {
  padding-top: 4px;
}

.dialog-delete-content {
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  padding: 4px 0;
}
</style>
