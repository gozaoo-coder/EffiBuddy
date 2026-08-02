<script setup lang="ts">
/**
 * ProviderPanel AI 服务商面板
 *
 * 布局：三级左栏 folder + 主内容区
 * - 三级左栏：按 ModelKind 分组（对话 / 生图 / 生视频 / 音频转文字），点击切换主内容区
 * - 主内容区：
 *   - 列表视图（默认）：显示当前 folder 下所有已配置模型 + 添加按钮
 *   - 表单视图：新建或编辑模型（ProviderModelForm 子组件）
 *
 * 数据流：
 * - onMounted 加载 config + presets
 * - 监听 agent-backend-changed 事件：外部（manage_model 工具）修改后刷新
 * - 保存/删除/激活模型通过 invoke 调用后端命令，刷新 config 后视图同步
 * - emit saved 通知父组件（ModelSettingsContent → App.vue）刷新后端信息
 *
 * 与旧 ModelConfigPanel 的差异：
 * - 不再使用 BindSheet 弹窗，直接嵌入主内容区
 * - 按 ModelKind 分组的 folder 替代原 SegmentedButton 的 list/form 切换
 * - 表单作为子组件 ProviderModelForm 独立，减少本文件复杂度
 */
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  Button,
  IconButton,
  Icon,
  Menu,
  type MenuItemOption,
  useToast,
  useSnackbar,
} from '../basic'
import { useAnimeTransition } from '../../composables/useAnimeTransition'
import ProviderModelForm from './ProviderModelForm.vue'
import type { AgentConfig, AvailableModel, ModelKind, ProviderPreset } from '../../types'

const emit = defineEmits<{ (e: 'saved'): void }>()
const { toast } = useToast()
const { snackbar } = useSnackbar()

// ---------- 数据 ----------
const config = ref<AgentConfig | null>(null)
const presets = ref<ProviderPreset[]>([])

// ---------- 三级 folder 配置 ----------
interface FolderMeta {
  key: ModelKind
  label: string
  icon: string
  desc: string
  accent: string
}
const folders: FolderMeta[] = [
  { key: 'chat', label: '对话', icon: 'chat', desc: 'LLM 文本对话与推理', accent: '#4a7eff' },
  { key: 'image_gen', label: '生成图片', icon: 'image', desc: 'DALL-E / SD / Flux', accent: '#a855f7' },
  { key: 'video_gen', label: '生成视频', icon: 'camera', desc: '文生视频（预留）', accent: '#f59e0b' },
  { key: 'audio_transcribe', label: '音频转文字', icon: 'mic', desc: 'Whisper 等转写', accent: '#10b981' },
]
const activeFolder = ref<ModelKind>('chat')
const activeFolderMeta = computed(() => folders.find((f) => f.key === activeFolder.value)!)

// ---------- 视图切换：list / form ----------
type View = 'list' | 'form'
const view = ref<View>('list')
const editingModel = ref<AvailableModel | null>(null)

const { onEnter, onLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    translateY: [10, 0],
    duration: 260,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    translateY: [0, -6],
    duration: 180,
    ease: 'inOut(2)',
  },
})

// ---------- 模型列表（按 folder 过滤） ----------
const folderModels = computed<AvailableModel[]>(() => {
  if (!config.value) return []
  return config.value.models.filter((m) => (m.kind ?? 'chat') === activeFolder.value)
})

const hasModels = computed(() => folderModels.value.length > 0)

// 当前 folder 对应的激活模型 id（用于列表高亮）
// active_image_gen_model_id 等字段是 optional，可能为 undefined，统一归一化为 null
const activeModelIdForFolder = computed<string | null>(() => {
  if (!config.value) return null
  switch (activeFolder.value) {
    case 'chat':
      return config.value.active_model_id
    case 'image_gen':
      return config.value.active_image_gen_model_id ?? null
    default:
      return null
  }
})

// ---------- Provider 视觉 ----------
interface ProviderVisual {
  glyph: string
  accent: string
}
const providerVisuals: Record<string, ProviderVisual> = {
  openai: { glyph: 'spark', accent: '#10a37f' },
  deepseek: { glyph: 'globe', accent: '#4d6bfe' },
  groq: { glyph: 'bolt', accent: '#f55036' },
  anthropic: { glyph: 'spark', accent: '#d97757' },
  moonshot: { glyph: 'moon', accent: '#16161a' },
  custom: { glyph: 'settings', accent: '#4a7eff' },
}
function visualOf(id: string): ProviderVisual {
  return providerVisuals[id] ?? { glyph: 'spark', accent: '#4a7eff' }
}

// ---------- 数据加载 ----------
let unlistenBackend: UnlistenFn | null = null
onMounted(async () => {
  await loadAll()
  unlistenBackend = await listen('agent-backend-changed', () => {
    invoke<AgentConfig>('get_config')
      .then((c) => {
        config.value = c
      })
      .catch(() => {})
  })
})
onUnmounted(() => {
  unlistenBackend?.()
  unlistenBackend = null
})

async function loadAll() {
  try {
    const [c, p] = await Promise.all([
      invoke<AgentConfig>('get_config'),
      invoke<ProviderPreset[]>('list_provider_presets'),
    ])
    config.value = c
    presets.value = p
  } catch (e) {
    toast({ content: `加载配置失败：${e}`, type: 'error' })
  }
}

// 切换 folder 时回到列表视图
watch(activeFolder, () => {
  view.value = 'list'
  editingModel.value = null
})

// ---------- 列表操作 ----------
function switchToCreate() {
  editingModel.value = null
  view.value = 'form'
}

function editModel(m: AvailableModel) {
  editingModel.value = m
  view.value = 'form'
}

function switchToList() {
  view.value = 'list'
  editingModel.value = null
}

// 保存（新建/编辑）：调用 save_model，刷新 config，回列表
async function onSaveModel(model: AvailableModel) {
  try {
    await invoke('save_model', { model })
    config.value = await invoke<AgentConfig>('get_config')
    toast({
      content: editingModel.value ? `已更新模型「${model.label}」` : `已保存模型「${model.label}」`,
      type: 'success',
    })
    emit('saved')
    switchToList()
  } catch (e) {
    toast({ content: `保存模型失败：${e}`, type: 'error' })
  }
}

// 激活模型：根据 kind 走不同命令
async function activateModel(id: string) {
  const m = config.value?.models.find((x) => x.id === id)
  if (!m) return
  const kind = m.kind ?? 'chat'
  try {
    if (kind === 'image_gen') {
      await invoke('set_image_gen_model', { id })
      config.value = await invoke<AgentConfig>('get_config')
      toast({ content: '已激活图像生成模型', type: 'success' })
    } else if (kind === 'chat') {
      await invoke('set_active_model', { id })
      config.value = await invoke<AgentConfig>('get_config')
      toast({ content: '已切换对话模型并热替换 agent', type: 'success' })
    } else {
      toast({ content: `${kind} 模型暂不支持直接激活，请在"服务模型"中配置`, type: 'warn' })
      return
    }
    emit('saved')
  } catch (e) {
    toast({ content: `激活失败：${e}`, type: 'error' })
  }
}

// 删除（带撤销）
async function deleteModel(id: string, label: string) {
  const snapshot = config.value
  try {
    await invoke('delete_model', { id })
    config.value = await invoke<AgentConfig>('get_config')
    snackbar({
      content: `已删除模型「${label}」`,
      mode: 'timed',
      action: {
        text: '撤销',
        onClick: async () => {
          if (snapshot) {
            const m = snapshot.models.find((x) => x.id === id)
            if (m) {
              try {
                await invoke('save_model', { model: m })
                config.value = await invoke<AgentConfig>('get_config')
                toast({ content: '已恢复', type: 'success' })
              } catch (e) {
                toast({ content: `恢复失败：${e}`, type: 'error' })
              }
            }
          }
        },
      },
    })
    emit('saved')
  } catch (e) {
    toast({ content: `删除失败：${e}`, type: 'error' })
  }
}

// ---------- 卡片菜单 ----------
const menuOpen = ref(false)
const menuTriggerEl = ref<HTMLElement | null>(null)
const menuModel = ref<AvailableModel | null>(null)

function openModelMenu(m: AvailableModel, e: MouseEvent) {
  menuModel.value = m
  menuTriggerEl.value = e.currentTarget as HTMLElement
  menuOpen.value = true
}

const menuItems = computed<MenuItemOption[]>(() => {
  const m = menuModel.value
  const isActive = !!m && activeModelIdForFolder.value === m.id
  const canActivate =
    !!m && (m.kind === 'chat' || m.kind === 'image_gen')
  return [
    { key: 'edit', label: '编辑', icon: 'edit' },
    {
      key: 'default',
      label: isActive ? '当前已激活' : '设为激活',
      icon: 'star',
      divided: true,
      disabled: isActive || !canActivate,
    },
    { key: 'delete', label: '删除', icon: 'delete', danger: true, divided: true },
  ]
})

function onMenuSelect(item: MenuItemOption) {
  const m = menuModel.value
  if (!m) return
  if (item.key === 'edit') editModel(m)
  else if (item.key === 'default') activateModel(m.id)
  else if (item.key === 'delete') deleteModel(m.id, m.label)
}
</script>

<template>
  <div class="pp">
    <!-- 三级左栏：folder 列表 -->
    <nav class="pp-rail">
      <header class="pp-rail-head">
        <span class="pp-rail-title">
          <Icon name="globe" :size="14" />
          AI 服务商
        </span>
      </header>
      <div class="pp-rail-list">
        <button
          v-for="f in folders"
          :key="f.key"
          type="button"
          class="pp-folder"
          :class="{ active: activeFolder === f.key }"
          @click="activeFolder = f.key"
        >
          <span class="pp-folder-glyph" :style="{ background: f.accent }">
            <Icon :name="f.icon" :size="16" />
          </span>
          <span class="pp-folder-info">
            <span class="pp-folder-label">{{ f.label }}</span>
            <span class="pp-folder-desc">{{ f.desc }}</span>
          </span>
          <span
            v-if="config?.models.filter((m) => (m.kind ?? 'chat') === f.key).length"
            class="pp-folder-count"
          >
            {{ config!.models.filter((m) => (m.kind ?? 'chat') === f.key).length }}
          </span>
        </button>
      </div>
      <footer class="pp-rail-foot">
        <p class="pp-rail-hint">
          <Icon name="info" :size="12" />
          按能力类型组织模型
        </p>
      </footer>
    </nav>

    <!-- 主内容区：列表视图 ↔ 表单视图 -->
    <section class="pp-main">
      <Transition :css="false" @enter="onEnter" @leave="onLeave" mode="out-in">
        <!-- ========== 列表视图 ========== -->
        <div v-if="view === 'list'" key="list" class="pp-list-view">
          <header class="pp-list-head">
            <div class="pp-list-title">
              <span class="pp-list-glyph" :style="{ background: activeFolderMeta.accent }">
                <Icon :name="activeFolderMeta.icon" :size="18" />
              </span>
              <div class="pp-list-title-text">
                <h2 class="pp-list-h">{{ activeFolderMeta.label }}</h2>
                <p class="pp-list-sub">{{ activeFolderMeta.desc }}</p>
              </div>
            </div>
            <Button variant="primary" size="sm" @click="switchToCreate">
              <Icon name="plus" :size="14" />
              添加模型
            </Button>
          </header>

          <div class="pp-list-body">
            <!-- 空状态 -->
            <div v-if="!hasModels" class="pp-empty">
              <div class="pp-empty-glyph" :style="{ color: activeFolderMeta.accent }">
                <Icon :name="activeFolderMeta.icon" :size="40" />
              </div>
              <p class="pp-empty-text">还没有{{ activeFolderMeta.label }}模型</p>
              <p class="pp-empty-hint">点击右上角"添加模型"，配置你的第一个模型</p>
              <Button variant="primary" size="md" class="pp-empty-action" @click="switchToCreate">
                <Icon name="plus" :size="14" />
                立即添加
              </Button>
            </div>

            <!-- 模型卡片列表 -->
            <div v-else class="pp-cards">
              <div
                v-for="m in folderModels"
                :key="m.id"
                class="pp-card"
                :class="{ active: activeModelIdForFolder === m.id }"
              >
                <div class="pp-card-main" @click="activateModel(m.id)">
                  <span
                    class="pp-card-glyph"
                    :style="{ background: visualOf(m.provider_id).accent }"
                  >
                    <Icon :name="visualOf(m.provider_id).glyph" :size="18" />
                  </span>
                  <div class="pp-card-info">
                    <div class="pp-card-top">
                      <span class="pp-card-label">{{ m.label }}</span>
                      <span
                        v-if="activeModelIdForFolder === m.id"
                        class="pp-active-pill"
                        :class="`pp-active-pill--${activeFolder}`"
                      >已激活</span>
                    </div>
                    <div class="pp-card-meta">
                      {{ m.provider_id }} · {{ m.model_name }}
                      <template v-if="m.kind === 'image_gen' && m.image_size">
                        <span class="pp-meta-sep">·</span>
                        <span>{{ m.image_size }}</span>
                      </template>
                      <template v-if="m.kind === 'video_gen'">
                        <template v-if="m.video_resolution">
                          <span class="pp-meta-sep">·</span>
                          <span>{{ m.video_resolution }}</span>
                        </template>
                        <template v-if="m.video_duration">
                          <span class="pp-meta-sep">·</span>
                          <span>{{ m.video_duration }}s</span>
                        </template>
                      </template>
                      <template v-if="m.kind === 'audio_transcribe' && m.audio_language">
                        <span class="pp-meta-sep">·</span>
                        <span>lang: {{ m.audio_language }}</span>
                      </template>
                    </div>
                  </div>
                </div>
                <IconButton
                  class="pp-card-menu"
                  title="更多操作"
                  @click="(e) => openModelMenu(m, e)"
                >
                  <Icon name="more" :size="18" />
                </IconButton>
              </div>
            </div>
          </div>
        </div>

        <!-- ========== 表单视图 ========== -->
        <ProviderModelForm
          v-else
          key="form"
          :initial="editingModel"
          :locked-kind="activeFolder"
          :presets="presets"
          :is-editing="!!editingModel"
          @save="onSaveModel"
          @cancel="switchToList"
        />
      </Transition>
    </section>

    <!-- 模型卡片操作菜单 -->
    <Menu
      :visible="menuOpen"
      :items="menuItems"
      :trigger-ref="menuTriggerEl"
      placement="bottom-end"
      @update:visible="menuOpen = $event"
      @select="onMenuSelect"
    />
  </div>
</template>

<style scoped>
.pp {
  flex: 1;
  display: flex;
  min-width: 0;
  overflow: hidden;
  background: var(--bg);
}

/* ---------- 三级左栏 ---------- */
.pp-rail {
  display: flex;
  flex-direction: column;
  width: 200px;
  flex-shrink: 0;
  background: var(--bg-2);
  border-right: 1px solid var(--border);
  user-select: none;
  overflow: hidden;
}

.pp-rail-head {
  display: flex;
  align-items: center;
  padding: 12px 14px 8px;
  border-bottom: 1px solid var(--border);
}

.pp-rail-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  letter-spacing: 0.2px;
}

.pp-rail-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 8px;
  flex: 1;
  overflow-y: auto;
}

.pp-folder {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 10px;
  border: 1px solid transparent;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
  outline: none;
  transition: background var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard);
  font-family: inherit;
}

.pp-folder:hover {
  background: var(--card);
}

.pp-folder.active {
  background: var(--card);
  border-color: var(--border);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.04);
}

.pp-folder:active {
  transform: scale(0.99);
}

.pp-folder-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: var(--radius-sm);
  color: #fff;
  flex-shrink: 0;
}

.pp-folder-info {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1;
}

.pp-folder-label {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pp-folder-desc {
  font-size: 10px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pp-folder-count {
  flex-shrink: 0;
  min-width: 22px;
  height: 18px;
  padding: 0 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: var(--fs-xs);
  color: var(--muted);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.pp-folder.active .pp-folder-count {
  color: var(--primary);
  background: rgba(74, 126, 255, 0.12);
  border-color: rgba(74, 126, 255, 0.3);
}

.pp-rail-foot {
  padding: 10px 12px;
  border-top: 1px solid var(--border);
}

.pp-rail-hint {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0;
  font-size: 10px;
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 主内容区 ---------- */
.pp-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* ---------- 列表视图 ---------- */
.pp-list-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

.pp-list-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 20px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.pp-list-title {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
}

.pp-list-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--radius);
  color: #fff;
  flex-shrink: 0;
}

.pp-list-title-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.pp-list-h {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 700;
  color: var(--text);
  letter-spacing: 0.2px;
}

.pp-list-sub {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
}

.pp-list-body {
  flex: 1;
  overflow-y: auto;
  padding: 16px 20px 28px;
}

/* ---------- 空状态 ---------- */
.pp-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 60px 20px;
  border: 1px dashed var(--border);
  border-radius: var(--radius);
  background: var(--card);
  text-align: center;
}

.pp-empty-glyph {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: 50%;
  background: var(--card-2);
  margin-bottom: 8px;
}

.pp-empty-text {
  margin: 0;
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.pp-empty-hint {
  margin: 0 0 12px;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.5;
  max-width: 320px;
}

.pp-empty-action {
  margin-top: 4px;
}

/* ---------- 模型卡片 ---------- */
.pp-cards {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.pp-card {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--card);
  transition: border-color var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard),
    box-shadow var(--duration-fast) var(--ease-standard);
}

.pp-card:hover {
  border-color: var(--primary);
}

.pp-card.active {
  border-color: var(--primary);
  background: rgba(74, 126, 255, 0.06);
  box-shadow: 0 0 0 1px var(--primary);
}

.pp-card-main {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 1;
  min-width: 0;
  cursor: pointer;
}

.pp-card-glyph {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border-radius: var(--radius-sm);
  color: #fff;
  flex-shrink: 0;
}

.pp-card-info {
  flex: 1;
  min-width: 0;
}

.pp-card-top {
  display: flex;
  align-items: center;
  gap: 8px;
}

.pp-card-label {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pp-active-pill {
  flex-shrink: 0;
  padding: 1px 8px;
  font-size: var(--fs-xs);
  color: #fff;
  border-radius: var(--radius-full);
  font-weight: 500;
}

.pp-active-pill--chat {
  background: var(--primary);
}

.pp-active-pill--image_gen {
  background: #a855f7;
}

.pp-card-meta {
  font-size: var(--fs-xs);
  color: var(--muted);
  margin-top: 3px;
  font-family: 'SFMono-Regular', Consolas, monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pp-meta-sep {
  margin: 0 4px;
  opacity: 0.5;
}

.pp-card-menu {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--muted);
  font-size: 18px;
  line-height: 1;
  cursor: pointer;
  flex-shrink: 0;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.pp-card-menu:hover {
  background: var(--card-2);
  color: var(--text);
}

/* ---------- 响应式 ---------- */
@media (max-width: 720px) {
  .pp-rail {
    width: 160px;
  }
  .pp-list-head,
  .pp-list-body {
    padding-left: 14px;
    padding-right: 14px;
  }
}
</style>
