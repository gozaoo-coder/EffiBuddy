<script setup lang="ts">
/**
 * ModelConfigPanel 可用模型配置面板
 *
 * 视图切换方案（替代旧版 Step1/2/3 三段式）：
 * - 顶部 SegmentedButton 在两个视图间切换：
 *   - list：我的模型（按 kind 分组展示已保存模型）
 *   - form：添加新模型 / 编辑现有模型（紧凑单页表单）
 * - list 视图右上角"添加"按钮 → 跳到 form 视图（新建模式）
 * - list 视图卡片菜单"编辑" → 跳到 form 视图（编辑模式，载入 draft）
 * - form 视图顶部"返回列表"链接 → 回到 list 视图
 *
 * 设计原则：
 * - 列表优先：默认进入"我的模型"列表，新建/编辑是显式动作
 * - 按 kind 分组：对话模型与图像生成模型分别展示，并显示各自激活态
 * - 表单不分步骤：所有字段在一个滚动页内，preamble 等次要字段折叠
 * - 切换视图使用淡入动画，避免突变
 */
import { ref, watch, onMounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  BindSheet,
  Button,
  IconButton,
  Icon,
  Switch,
  Dropdown,
  Menu,
  SegmentedButton,
  type DropdownOption,
  type MenuItemOption,
  type SegmentedOption,
  useToast,
  useSnackbar,
} from './basic'
import { useAnimeTransition } from '../composables/useAnimeTransition'
import type { AgentConfig, AvailableModel, ProviderPreset, RemoteModelInfo, BackendKind, ModelKind } from '../types'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void; (e: 'saved', backend: string): void }>()

const { toast } = useToast()
const { snackbar } = useSnackbar()

// 完整配置（含 models 列表与 active_model_id）
const config = ref<AgentConfig | null>(null)
// 内置 provider 预设
const presets = ref<ProviderPreset[]>([])

// ---------- 顶层视图切换：list / form ----------
type View = 'list' | 'form'
const view = ref<View>('list')

const viewOptions: SegmentedOption[] = [
  { label: '我的模型', value: 'list' },
  { label: '添加新模型', value: 'form' },
]

// 视图切换淡入动画
const { onEnter, onLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    translateY: [10, 0],
    duration: 280,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    translateY: [0, -8],
    duration: 180,
    ease: 'inOut(2)',
  },
})

// 当前编辑的 draft
const draft = ref({
  backend: 'openai' as BackendKind,
  provider_id: '',
  api_key: '',
  base_url: '',
  model_name: '',
  preamble: '你是 EffiSuite 的 AI 助手，简洁友好地回答用户问题。',
  enable_tools: true,
  // 模型能力类型：chat（对话）/ image_gen（图像生成）
  kind: 'chat' as ModelKind,
  // 图像生成专用字段
  image_size: '' as string,
  image_quality: '' as string,
  // 上下文窗口大小（tokens）
  context_window_tokens: 128000 as number,
})
// 当前编辑的模型 id（非空表示编辑模式）
const editingId = ref<string | null>(null)
// 保存为模型时的标签
const saveLabel = ref('')
const saving = ref(false)

// API Key 显隐 / Preamble 折叠
const showApiKey = ref(false)
const preambleExpanded = ref(false)

// 模型类型选项
const kindOptions: { value: ModelKind; label: string; icon: string; desc: string }[] = [
  { value: 'chat', label: '对话模型', icon: 'chat', desc: 'LLM 文本对话与推理' },
  { value: 'image_gen', label: '图像生成', icon: 'image', desc: 'DALL-E / SD / Flux 等文生图' },
]

// 图像尺寸预设
const imageSizePresets = ['1024x1024', '1792x1024', '1024x1792', '512x512', '256x256']
const imageQualityPresets = ['standard', 'hd']

const isImageGen = computed(() => draft.value.kind === 'image_gen')
const isEditing = computed(() => !!editingId.value)

// ---------- provider 视觉映射 ----------
interface ProviderVisual {
  glyph: string
  accent: string
  desc: string
}

const providerVisuals: Record<string, ProviderVisual> = {
  openai: { glyph: 'spark', accent: '#10a37f', desc: 'GPT 系列模型，通用能力强' },
  deepseek: { glyph: 'globe', accent: '#4d6bfe', desc: '高性价比推理与对话' },
  groq: { glyph: 'bolt', accent: '#f55036', desc: '超低延迟推理加速' },
  anthropic: { glyph: 'spark', accent: '#d97757', desc: 'Claude 系列长文本模型' },
  moonshot: { glyph: 'moon', accent: '#16161a', desc: 'Kimi 长上下文模型' },
  custom: { glyph: 'settings', accent: '#4a7eff', desc: '任意 OpenAI 兼容端点' },
}

function visualOf(id: string): ProviderVisual {
  return providerVisuals[id] ?? { glyph: 'spark', accent: '#4a7eff', desc: '自定义模型端点' }
}

// 推荐模型列表（部分 provider）
const recommendedModels: Record<string, string[]> = {
  openai: ['gpt-4o-mini', 'gpt-4o', 'gpt-4-turbo', 'gpt-3.5-turbo'],
  deepseek: ['deepseek-chat', 'deepseek-reasoner'],
  groq: ['llama-3.3-70b-versatile', 'llama-3.1-8b-instant', 'mixtral-8x7b-32768'],
  anthropic: ['claude-3-5-sonnet-20241022', 'claude-3-5-haiku-20241022'],
  moonshot: ['moonshot-v1-8k', 'moonshot-v1-32k', 'moonshot-v1-128k'],
}

// 从 API 实际拉取的可用模型列表（用户点"从 API 获取"按钮触发）
// 优先级高于 recommendedModels：有值时 Dropdown 显示 API 列表
const remoteModels = ref<RemoteModelInfo[]>([])
const fetchingModels = ref(false)

const modelDropdownOptions = computed<DropdownOption[]>(() => {
  // 优先用 API 拉取的列表
  if (remoteModels.value.length > 0) {
    return remoteModels.value.map((m) => ({
      label: m.id + (m.owned_by ? ` · ${m.owned_by}` : ''),
      value: m.id,
    }))
  }
  // 退回到硬编码推荐列表
  const list = recommendedModels[draft.value.provider_id]
  if (!list) return []
  return list.map((m) => ({ label: m, value: m }))
})

const hasModelRecommendations = computed(() => modelDropdownOptions.value.length > 0)

// 从 API 拉取可用模型列表
// 要求用户已填 base_url 与 api_key，否则提示
async function fetchRemoteModels() {
  if (!draft.value.base_url.trim()) {
    toast({ content: '请先填写 Base URL', type: 'warn' })
    return
  }
  if (!draft.value.api_key.trim()) {
    toast({ content: '请先填写 API Key', type: 'warn' })
    return
  }
  fetchingModels.value = true
  try {
    const list = await invoke<RemoteModelInfo[]>('list_remote_models', {
      baseUrl: draft.value.base_url,
      apiKey: draft.value.api_key,
    })
    remoteModels.value = list
    if (list.length === 0) {
      toast({ content: 'API 返回空列表', type: 'warn' })
    } else {
      toast({ content: `已获取 ${list.length} 个可用模型`, type: 'success' })
    }
  } catch (e) {
    toast({ content: `拉取模型失败：${e}`, type: 'error' })
    remoteModels.value = []
  } finally {
    fetchingModels.value = false
  }
}

// ---------- 数据加载 ----------
onMounted(async () => {
  await loadAll()
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

watch(
  () => props.open,
  (v) => {
    if (v) {
      invoke<AgentConfig>('get_config')
        .then((c) => {
          config.value = c
        })
        .catch((e) => toast({ content: `加载失败：${e}`, type: 'error' }))
      // 每次打开默认进入 list 视图，避免上一次编辑残留
      if (!isEditing.value) view.value = 'list'
    }
  },
)

function findPreset(id: string): ProviderPreset | undefined {
  return presets.value.find((p) => p.id === id)
}

// ---------- 进入 form 视图 ----------
function switchToCreate() {
  resetDraft()
  view.value = 'form'
}

function switchToList() {
  view.value = 'list'
  // 离开 form 时若是新建未保存，清掉 draft；编辑态保留以便再次进入
  if (!isEditing.value) resetDraft()
}

function resetDraft() {
  draft.value = {
    backend: 'openai',
    provider_id: '',
    api_key: '',
    base_url: '',
    model_name: '',
    preamble: '你是 EffiSuite 的 AI 助手，简洁友好地回答用户问题。',
    enable_tools: true,
    kind: 'chat',
    image_size: '',
    image_quality: '',
    context_window_tokens: 128000,
  }
  editingId.value = null
  saveLabel.value = ''
  showApiKey.value = false
  preambleExpanded.value = false
  // 清空 API 拉取的模型列表，避免残留
  remoteModels.value = []
}

// ---------- provider 选择 ----------
function selectPreset(p: ProviderPreset) {
  draft.value.provider_id = p.id
  if (p.id !== 'custom') {
    draft.value.base_url = p.default_base_url
    if (p.default_model) draft.value.model_name = p.default_model
  }
  draft.value.backend = 'openai'
  editingId.value = null
  // 切换 provider 后清空 API 拉取列表（不同 provider 模型不同）
  remoteModels.value = []
}

function onModelNameDropdownChange(v: string | number, _opt: DropdownOption) {
  draft.value.model_name = String(v)
}

// ---------- 保存为可用模型 ----------
function newId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID()
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

async function saveAsModel() {
  if (!config.value) return
  const label = saveLabel.value.trim()
  if (!label) {
    toast({ content: '请输入模型标签', type: 'warn' })
    return
  }
  if (!draft.value.api_key && draft.value.provider_id !== 'custom') {
    toast({ content: '请填写 API Key', type: 'warn' })
    return
  }
  saving.value = true
  const wasEditing = !!editingId.value
  try {
    const id = editingId.value ?? newId()
    const model: AvailableModel = {
      id,
      label,
      provider_id: draft.value.provider_id,
      base_url: draft.value.base_url,
      model_name: draft.value.model_name,
      api_key: draft.value.api_key,
      preamble: draft.value.preamble,
      enable_tools: draft.value.enable_tools,
      kind: draft.value.kind,
      // 图像生成专用字段：仅 kind=image_gen 时有值，否则传 null 让后端忽略
      image_size:
        draft.value.kind === 'image_gen' && draft.value.image_size.trim()
          ? draft.value.image_size.trim()
          : null,
      image_quality:
        draft.value.kind === 'image_gen' && draft.value.image_quality.trim()
          ? draft.value.image_quality.trim()
          : null,
      context_window_tokens: draft.value.context_window_tokens || null,
      created_at: Date.now(),
    }
    await invoke('save_model', { model })
    toast({
      content: wasEditing ? `已更新模型「${label}」` : `已保存模型「${label}」`,
      type: 'success',
    })
    config.value = await invoke<AgentConfig>('get_config')
    // 保存成功后回到列表视图
    resetDraft()
    view.value = 'list'
  } catch (e) {
    toast({ content: `保存模型失败：${e}`, type: 'error' })
  } finally {
    saving.value = false
  }
}

// 编辑已有模型：载入 draft 并跳到 form 视图
function editModel(m: AvailableModel) {
  draft.value = {
    backend: 'openai',
    provider_id: m.provider_id,
    api_key: m.api_key,
    base_url: m.base_url,
    model_name: m.model_name,
    preamble: m.preamble,
    enable_tools: m.enable_tools,
    kind: m.kind ?? 'chat',
    image_size: m.image_size ?? '',
    image_quality: m.image_quality ?? '',
    context_window_tokens: m.context_window_tokens ?? 128000,
  }
  editingId.value = m.id
  saveLabel.value = m.label
  preambleExpanded.value = true
  showApiKey.value = false
  view.value = 'form'
}

// ---------- 激活模型 ----------
// 根据 model.kind 走不同命令：
// - chat：set_active_model 重建对话 agent
// - image_gen：set_image_gen_model 更新图像生成配置（不重建对话 agent）
async function activateModel(id: string) {
  const m = config.value?.models.find((x) => x.id === id)
  const kind = m?.kind ?? 'chat'
  try {
    if (kind === 'image_gen') {
      await invoke('set_image_gen_model', { id })
      config.value = await invoke<AgentConfig>('get_config')
      toast({ content: '已激活图像生成模型', type: 'success' })
    } else {
      await invoke('set_active_model', { id })
      config.value = await invoke<AgentConfig>('get_config')
      toast({ content: '已切换对话模型并热替换 agent', type: 'success' })
    }
    emit('saved', 'rig-openai-compat')
  } catch (e) {
    toast({ content: `激活失败：${e}`, type: 'error' })
  }
}

// ---------- 删除模型（带撤销） ----------
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
  } catch (e) {
    toast({ content: `删除失败：${e}`, type: 'error' })
  }
}

// ---------- 模型卡片菜单 ----------
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
  const isActive =
    !!m &&
    (config.value?.active_model_id === m.id || config.value?.active_image_gen_model_id === m.id)
  return [
    { key: 'edit', label: '编辑', icon: 'edit' },
    {
      key: 'default',
      label: isActive ? '当前已激活' : '设为激活',
      icon: 'star',
      divided: true,
      disabled: isActive,
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

// ---------- 列表分组与统计 ----------
const hasModels = computed(() => !!config.value && config.value.models.length > 0)

// 按 kind 分组：chat / image_gen
const chatModels = computed<AvailableModel[]>(
  () => config.value?.models.filter((m) => (m.kind ?? 'chat') === 'chat') ?? [],
)
const imageModels = computed<AvailableModel[]>(
  () => config.value?.models.filter((m) => m.kind === 'image_gen') ?? [],
)

const activeChatModel = computed(
  () =>
    config.value?.models.find((m) => m.id === config.value?.active_model_id) ?? null,
)
const activeImageModel = computed(
  () =>
    config.value?.models.find((m) => m.id === config.value?.active_image_gen_model_id) ??
    null,
)

function onClose() {
  emit('close')
}
</script>

<template>
  <BindSheet
    :visible="props.open"
    side="right"
    width="620px"
    title="可用模型配置"
    @close="onClose"
  >
    <div class="mcp-body">
      <!-- 顶部 SegmentedButton 视图切换 -->
      <div class="view-switch">
        <SegmentedButton
          v-model="view"
          :options="viewOptions"
          size="md"
          block
        />
      </div>

      <Transition :css="false" @enter="onEnter" @leave="onLeave" mode="out-in">
        <!-- ========== 视图 1：我的模型（按 kind 分组） ========== -->
        <section v-if="view === 'list'" key="list" class="view-page">
          <!-- 顶部操作条 -->
          <div class="list-toolbar">
            <div class="list-summary">
              <span class="summary-chip">
                <Icon name="chat" :size="14" />
                对话 {{ chatModels.length }}
              </span>
              <span class="summary-chip summary-chip--image">
                <Icon name="image" :size="14" />
                图像 {{ imageModels.length }}
              </span>
            </div>
            <Button variant="primary" size="sm" @click="switchToCreate">
              <Icon name="plus" :size="14" />
              添加模型
            </Button>
          </div>

          <!-- 空状态 -->
          <div v-if="!hasModels" class="empty-state">
            <div class="empty-illust">✦</div>
            <p class="empty-text">还没有可用模型</p>
            <p class="empty-hint">点击右上角"添加模型"，配置你的第一个 AI 模型</p>
            <Button variant="primary" size="md" class="empty-action" @click="switchToCreate">
              <Icon name="plus" :size="14" />
              立即添加
            </Button>
          </div>

          <!-- 列表分组 -->
          <template v-else>
            <!-- 对话模型组 -->
            <div class="kind-group">
              <header class="kind-group-head">
                <span class="kind-group-title">
                  <Icon name="chat" :size="16" />
                  对话模型
                  <span class="kind-group-count">{{ chatModels.length }}</span>
                </span>
                <span v-if="activeChatModel" class="kind-group-active">
                  当前激活：{{ activeChatModel.label }}
                </span>
                <span v-else class="kind-group-active kind-group-active--none">
                  未激活
                </span>
              </header>

              <div v-if="chatModels.length > 0" class="model-list">
                <div
                  v-for="m in chatModels"
                  :key="m.id"
                  class="model-card"
                  :class="{ active: config!.active_model_id === m.id }"
                >
                  <div class="model-card-main" @click="activateModel(m.id)">
                    <span
                      class="model-card-glyph"
                      :style="{ background: visualOf(m.provider_id).accent }"
                    >
                      <Icon :name="visualOf(m.provider_id).glyph" :size="20" />
                    </span>
                    <div class="model-card-info">
                      <div class="model-card-top">
                        <span class="model-card-label">{{ m.label }}</span>
                        <span
                          v-if="config!.active_model_id === m.id"
                          class="active-pill"
                        >已激活</span>
                      </div>
                      <div class="model-card-meta">
                        {{ m.provider_id }} · {{ m.model_name }}
                      </div>
                    </div>
                  </div>
                  <IconButton
                    class="model-card-menu"
                    title="更多操作"
                    @click="(e) => openModelMenu(m, e)"
                  >
                    <Icon name="more" :size="20" />
                  </IconButton>
                </div>
              </div>
              <p v-else class="group-empty">暂无对话模型，点击"添加模型"创建</p>
            </div>

            <!-- 图像生成模型组 -->
            <div class="kind-group">
              <header class="kind-group-head">
                <span class="kind-group-title">
                  <Icon name="image" :size="16" />
                  图像生成模型
                  <span class="kind-group-count">{{ imageModels.length }}</span>
                </span>
                <span v-if="activeImageModel" class="kind-group-active kind-group-active--image">
                  当前激活：{{ activeImageModel.label }}
                </span>
                <span v-else class="kind-group-active kind-group-active--none">
                  未激活
                </span>
              </header>

              <div v-if="imageModels.length > 0" class="model-list">
                <div
                  v-for="m in imageModels"
                  :key="m.id"
                  class="model-card"
                  :class="{ active: config!.active_image_gen_model_id === m.id }"
                >
                  <div class="model-card-main" @click="activateModel(m.id)">
                    <span
                      class="model-card-glyph model-card-glyph--image"
                      :style="{ background: visualOf(m.provider_id).accent }"
                    >
                      <Icon name="image" :size="20" />
                    </span>
                    <div class="model-card-info">
                      <div class="model-card-top">
                        <span class="model-card-label">{{ m.label }}</span>
                        <span class="kind-pill kind-pill--image">图像</span>
                        <span
                          v-if="config!.active_image_gen_model_id === m.id"
                          class="active-pill active-pill--image"
                        >已激活</span>
                      </div>
                      <div class="model-card-meta">
                        {{ m.provider_id }} · {{ m.model_name }}
                        <span v-if="m.image_size" class="meta-sep">·</span>
                        <span v-if="m.image_size">{{ m.image_size }}</span>
                      </div>
                    </div>
                  </div>
                  <IconButton
                    class="model-card-menu"
                    title="更多操作"
                    @click="(e) => openModelMenu(m, e)"
                  >
                    <Icon name="more" :size="20" />
                  </IconButton>
                </div>
              </div>
              <p v-else class="group-empty">暂无图像生成模型，对话中调用 image_gen 工具需要先配置</p>
            </div>
          </template>
        </section>

        <!-- ========== 视图 2：添加/编辑模型表单 ========== -->
        <section v-else key="form" class="view-page">
          <!-- 表单顶部：返回 + 标题 -->
          <header class="form-head">
            <button type="button" class="back-link" @click="switchToList">
              <Icon name="chevron-right" :size="14" class="back-icon" />
              返回列表
            </button>
            <h3 class="form-title">
              {{ isEditing ? `编辑：${saveLabel || '未命名模型'}` : '添加新模型' }}
            </h3>
            <span v-if="isEditing" class="editing-tag">编辑中</span>
          </header>

          <!-- 模型类型选择 -->
          <div class="field">
            <label class="field-label">模型类型</label>
            <div class="kind-grid">
              <button
                v-for="k in kindOptions"
                :key="k.value"
                type="button"
                class="kind-card"
                :class="{ selected: draft.kind === k.value }"
                @click="draft.kind = k.value"
              >
                <span class="kind-glyph"><Icon :name="k.icon" :size="18" /></span>
                <span class="kind-info">
                  <span class="kind-label">{{ k.label }}</span>
                  <span class="kind-desc">{{ k.desc }}</span>
                </span>
                <span v-if="draft.kind === k.value" class="kind-check">✓</span>
              </button>
            </div>
          </div>

          <!-- 服务商选择 -->
          <div class="field">
            <label class="field-label">服务商</label>
            <div class="provider-grid">
              <button
                v-for="p in presets"
                :key="p.id"
                type="button"
                class="provider-card"
                :class="{ selected: draft.provider_id === p.id }"
                @click="selectPreset(p)"
              >
                <span
                  class="provider-glyph"
                  :style="{ background: visualOf(p.id).accent }"
                ><Icon :name="visualOf(p.id).glyph" :size="18" /></span>
                <span class="provider-info">
                  <span class="provider-name">{{ p.name }}</span>
                </span>
                <span v-if="draft.provider_id === p.id" class="provider-check">✓</span>
              </button>
              <button
                v-if="!presets.some((p) => p.id === 'custom')"
                type="button"
                class="provider-card"
                :class="{ selected: draft.provider_id === 'custom' }"
                @click="selectPreset({ id: 'custom', name: '自定义', default_base_url: '', default_model: '', env_var: '', docs_url: '', openai_compat: true } as ProviderPreset)"
              >
                <span class="provider-glyph" :style="{ background: visualOf('custom').accent }">
                  <Icon :name="visualOf('custom').glyph" :size="18" />
                </span>
                <span class="provider-info">
                  <span class="provider-name">自定义</span>
                </span>
                <span v-if="draft.provider_id === 'custom'" class="provider-check">✓</span>
              </button>
            </div>
            <p v-if="findPreset(draft.provider_id)?.docs_url" class="provider-hint">
              <a :href="findPreset(draft.provider_id)!.docs_url" target="_blank" rel="noopener">
                📖 文档：{{ findPreset(draft.provider_id)!.docs_url }}
              </a>
              <span v-if="findPreset(draft.provider_id)!.env_var" class="env-tag">
                推荐 {{ findPreset(draft.provider_id)!.env_var }}
              </span>
            </p>
          </div>

          <!-- API Key -->
          <div class="field">
            <label class="field-label">API Key</label>
            <div class="input-with-action">
              <input
                v-model="draft.api_key"
                :type="showApiKey ? 'text' : 'password'"
                placeholder="sk-..."
                class="field-input"
              />
              <IconButton
                size="sm"
                container
                :title="showApiKey ? '隐藏' : '显示'"
                @click="showApiKey = !showApiKey"
              >
                <Icon :name="showApiKey ? 'eye-off' : 'eye'" :size="18" />
              </IconButton>
            </div>
          </div>

          <!-- Base URL + Model Name（两列紧凑布局） -->
          <div class="field-row-2col">
            <div class="field">
              <label class="field-label">
                Base URL
                <span v-if="draft.provider_id === 'custom'" class="hint-tag">自定义</span>
              </label>
              <input
                v-model="draft.base_url"
                type="text"
                placeholder="https://api.openai.com/v1"
                class="field-input"
              />
            </div>
            <div class="field">
              <label class="field-label">
                模型名
                <span
                  v-if="remoteModels.length > 0"
                  class="api-source-badge"
                  :title="`来自 API（${remoteModels.length} 个可用模型）`"
                >API</span>
              </label>
              <div class="model-name-row">
                <Dropdown
                  v-if="hasModelRecommendations && !isImageGen"
                  :model-value="draft.model_name"
                  :options="modelDropdownOptions"
                  :searchable="true"
                  :placeholder="remoteModels.length > 0 ? '搜索 API 模型...' : '选择或搜索推荐模型...'"
                  size="md"
                  class="model-name-dropdown"
                  @change="onModelNameDropdownChange"
                />
                <input
                  v-else
                  v-model="draft.model_name"
                  type="text"
                  :placeholder="isImageGen ? 'dall-e-3 / flux-pro / sd3' : 'gpt-4o-mini'"
                  class="field-input"
                />
                <button
                  type="button"
                  class="fetch-models-btn"
                  :disabled="fetchingModels || isImageGen"
                  :title="isImageGen ? '图像生成模型不支持拉取列表' : '从 API 拉取可用模型'"
                  @click="fetchRemoteModels"
                >
                  <span v-if="fetchingModels" class="fetch-spinner"></span>
                  <Icon v-else name="refresh" :size="16" />
                  <span class="fetch-btn-text">{{ fetchingModels ? '拉取中' : '从 API 获取' }}</span>
                </button>
              </div>
            </div>
          </div>

          <!-- 上下文窗口 -->
          <div class="field">
            <label class="field-label">上下文窗口（tokens）</label>
            <input
              v-model.number="draft.context_window_tokens"
              type="number"
              min="1024"
              step="1024"
              placeholder="128000"
              class="field-input"
            />
            <p class="field-hint">用于估算当前对话已用上下文比例</p>
          </div>

          <!-- 图像生成专用字段：尺寸与质量 -->
          <template v-if="isImageGen">
            <div class="field-row-2col">
              <div class="field">
                <label class="field-label">默认尺寸</label>
                <input
                  v-model="draft.image_size"
                  type="text"
                  placeholder="1024x1024（留空用默认）"
                  class="field-input"
                  list="image-size-list"
                />
                <datalist id="image-size-list">
                  <option v-for="s in imageSizePresets" :key="s" :value="s" />
                </datalist>
              </div>
              <div class="field">
                <label class="field-label">默认质量</label>
                <input
                  v-model="draft.image_quality"
                  type="text"
                  placeholder="standard / hd"
                  class="field-input"
                  list="image-quality-list"
                />
                <datalist id="image-quality-list">
                  <option v-for="q in imageQualityPresets" :key="q" :value="q" />
                </datalist>
              </div>
            </div>
          </template>

          <!-- 启用工具（仅对话模型有意义） -->
          <div v-if="!isImageGen" class="field field-row">
            <div class="field-row-text">
              <label class="field-label">启用工具调用</label>
              <span class="field-row-hint">RAG 历史检索 / 时间查询 / 图像生成</span>
            </div>
            <Switch v-model="draft.enable_tools" size="md" />
          </div>

          <!-- System Preamble（折叠） -->
          <div class="field">
            <button
              type="button"
              class="collapsible-head"
              :class="{ expanded: preambleExpanded }"
              @click="preambleExpanded = !preambleExpanded"
            >
              <span class="collapsible-label">系统提示词（preamble）</span>
              <span class="collapsible-arrow">
                <Icon :name="preambleExpanded ? 'chevron-down' : 'chevron-right'" :size="14" />
              </span>
            </button>
            <textarea
              v-if="preambleExpanded"
              v-model="draft.preamble"
              rows="4"
              placeholder="定义 agent 的人设与行为约束"
              class="field-input field-textarea"
            ></textarea>
          </div>

          <!-- 保存区 -->
          <div class="save-block">
            <label class="field-label">模型标签</label>
            <div class="save-row">
              <input
                v-model="saveLabel"
                type="text"
                :placeholder="isEditing ? '修改标签名' : '例如：我的 GPT-4o 工作号'"
                class="field-input"
              />
              <Button variant="primary" :loading="saving" @click="saveAsModel">
                {{ isEditing ? '更新并返回' : '保存并返回' }}
              </Button>
            </div>
          </div>
        </section>
      </Transition>
    </div>

    <!-- 模型卡片操作菜单 -->
    <Menu
      :visible="menuOpen"
      :items="menuItems"
      :trigger-ref="menuTriggerEl"
      placement="bottom-end"
      @update:visible="menuOpen = $event"
      @select="onMenuSelect"
    />
  </BindSheet>
</template>

<style scoped>
.mcp-body {
  padding: 20px 24px 32px;
  display: flex;
  flex-direction: column;
  gap: 16px;
  overflow-y: auto;
}

/* ---------- 顶部视图切换 ---------- */
.view-switch {
  position: sticky;
  top: 0;
  z-index: 5;
  background: var(--bg);
  padding: 4px 0 8px;
}

.view-page {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

/* ---------- 列表视图 ---------- */
.list-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.list-summary {
  display: flex;
  gap: 8px;
}

.summary-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  font-size: var(--fs-xs);
  color: var(--primary);
  background: rgba(74, 126, 255, 0.10);
  border: 1px solid rgba(74, 126, 255, 0.25);
  border-radius: var(--radius-full);
  font-weight: 500;
}

.summary-chip--image {
  color: #a855f7;
  background: rgba(168, 85, 247, 0.10);
  border-color: rgba(168, 85, 247, 0.25);
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
  margin: 0 0 8px;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.5;
  max-width: 320px;
}

.empty-action {
  margin-top: 4px;
}

/* ---------- kind 分组 ---------- */
.kind-group {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.kind-group-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 4px;
}

.kind-group-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.kind-group-count {
  padding: 1px 8px;
  font-size: var(--fs-xs);
  color: var(--muted);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.kind-group-active {
  font-size: var(--fs-xs);
  color: var(--primary);
  background: rgba(74, 126, 255, 0.08);
  padding: 2px 8px;
  border-radius: var(--radius-full);
}

.kind-group-active--image {
  color: #a855f7;
  background: rgba(168, 85, 247, 0.08);
}

.kind-group-active--none {
  color: var(--muted);
  background: var(--card-2);
}

.group-empty {
  margin: 0;
  padding: 12px 14px;
  font-size: var(--fs-sm);
  color: var(--muted);
  background: var(--card-2);
  border: 1px dashed var(--border);
  border-radius: var(--radius-sm);
  text-align: center;
}

/* ---------- 模型卡片 ---------- */
.model-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.model-card {
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

.model-card:hover {
  border-color: var(--primary);
}

.model-card.active {
  border-color: var(--primary);
  background: rgba(74, 126, 255, 0.08);
  box-shadow: 0 0 0 1px var(--primary);
}

.model-card-main {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 1;
  min-width: 0;
  cursor: pointer;
}

.model-card-glyph {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border-radius: var(--radius-sm);
  color: #fff;
  font-size: 17px;
  flex-shrink: 0;
}

.model-card-info {
  flex: 1;
  min-width: 0;
}

.model-card-top {
  display: flex;
  align-items: center;
  gap: 8px;
}

.model-card-label {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.active-pill {
  flex-shrink: 0;
  padding: 1px 8px;
  font-size: var(--fs-xs);
  color: #fff;
  background: var(--primary);
  border-radius: var(--radius-full);
}

.active-pill--image {
  background: #a855f7 !important;
}

.model-card-meta {
  font-size: var(--fs-xs);
  color: var(--muted);
  margin-top: 3px;
  font-family: 'SFMono-Regular', Consolas, monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.meta-sep {
  margin: 0 4px;
  opacity: 0.5;
}

.model-card-menu {
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

.model-card-menu:hover {
  background: var(--card-2);
  color: var(--text);
}

/* 模型类型 pill */
.kind-pill {
  flex-shrink: 0;
  padding: 1px 7px;
  font-size: var(--fs-xs);
  border-radius: var(--radius-full);
  font-weight: 500;
}

.kind-pill--image {
  color: #a855f7;
  background: rgba(168, 85, 247, 0.12);
}

/* ---------- 表单视图 ---------- */
.form-head {
  display: flex;
  align-items: center;
  gap: 12px;
  padding-bottom: 4px;
  border-bottom: 1px solid var(--border);
}

.back-link {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--muted);
  font-family: inherit;
  font-size: var(--fs-xs);
  cursor: pointer;
  transition: border-color var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.back-link:hover {
  border-color: var(--primary);
  color: var(--primary);
}

.back-icon {
  transform: rotate(180deg);
}

.form-title {
  flex: 1;
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.editing-tag {
  flex-shrink: 0;
  padding: 2px 8px;
  font-size: var(--fs-xs);
  color: var(--warn);
  background: rgba(240, 192, 74, 0.14);
  border: 1px solid rgba(240, 192, 74, 0.4);
  border-radius: var(--radius-full);
}

/* ---------- 表单字段 ---------- */
.field {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.field-label {
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text);
}

.field-input {
  width: 100%;
  height: var(--h-control-md);
  padding: 0 12px;
  font-family: inherit;
  font-size: var(--fs-base);
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
  box-sizing: border-box;
}

.field-input:focus {
  border-color: var(--primary);
}

.field-textarea {
  height: auto;
  min-height: 84px;
  padding: 10px 12px;
  resize: vertical;
  line-height: 1.5;
  font-family: inherit;
}

.field-row-2col {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}

/* ---------- 模型名行：Dropdown + 从 API 获取按钮 ---------- */
.model-name-row {
  display: flex;
  gap: 8px;
  align-items: stretch;
}

.model-name-dropdown {
  flex: 1;
  min-width: 0;
}

.model-name-row .field-input {
  flex: 1;
  min-width: 0;
}

/* API 来源徽标 */
.api-source-badge {
  display: inline-flex;
  align-items: center;
  padding: 1px 6px;
  margin-left: 6px;
  font-size: 10px;
  font-weight: 600;
  color: #fff;
  background: linear-gradient(135deg, #4a7eff, #6c5ce7);
  border-radius: 4px;
  letter-spacing: 0.5px;
}

/* 从 API 获取按钮 */
.fetch-models-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  border: 1px solid var(--border-color, rgba(0, 0, 0, 0.1));
  border-radius: 8px;
  background: var(--bg-elevated, rgba(0, 0, 0, 0.04));
  color: var(--text-secondary, inherit);
  cursor: pointer;
  font-size: 12px;
  font-weight: 500;
  white-space: nowrap;
  transition: all 0.15s ease;
}

.fetch-models-btn:hover:not(:disabled) {
  background: var(--bg-hover, rgba(74, 126, 255, 0.1));
  border-color: var(--accent, #4a7eff);
  color: var(--accent, #4a7eff);
}

.fetch-models-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.fetch-btn-text {
  line-height: 1;
}

/* 加载旋转动画 */
.fetch-spinner {
  width: 14px;
  height: 14px;
  border: 2px solid currentColor;
  border-top-color: transparent;
  border-radius: 50%;
  animation: fetch-spin 0.8s linear infinite;
}

@keyframes fetch-spin {
  to {
    transform: rotate(360deg);
  }
}

.input-with-action {
  display: flex;
  gap: 8px;
}

.input-with-action .field-input {
  flex: 1;
}

.hint-tag {
  display: inline-block;
  margin-left: 6px;
  padding: 1px 6px;
  font-size: var(--fs-xs);
  color: var(--warn);
  background: rgba(240, 192, 74, 0.12);
  border-radius: var(--radius-xs);
}

/* ---------- provider 卡片网格 ---------- */
.provider-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  gap: 8px;
}

.provider-card {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--card);
  cursor: pointer;
  text-align: left;
  outline: none;
  transition: transform var(--duration-base) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard),
    box-shadow var(--duration-fast) var(--ease-standard);
}

.provider-card:hover {
  border-color: var(--primary);
  background: var(--card-2);
}

.provider-card.selected {
  border-color: var(--primary);
  box-shadow: 0 0 0 1px var(--primary);
}

.provider-glyph {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: var(--radius-sm);
  color: #fff;
  flex-shrink: 0;
}

.provider-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.provider-name {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.provider-check {
  position: absolute;
  top: 6px;
  right: 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--primary);
  color: #fff;
  font-size: 10px;
  font-weight: 700;
}

.provider-hint {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-xs);
  color: var(--muted);
  margin: 0;
}

.provider-hint a {
  color: var(--primary);
  text-decoration: none;
}

.provider-hint a:hover {
  text-decoration: underline;
}

.env-tag {
  padding: 1px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  font-family: 'SFMono-Regular', Consolas, monospace;
}

/* ---------- 模型类型选择 ---------- */
.kind-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}

.kind-card {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--card);
  cursor: pointer;
  text-align: left;
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard),
    box-shadow var(--duration-fast) var(--ease-standard);
}

.kind-card:hover {
  border-color: var(--primary);
  background: var(--card-2);
}

.kind-card.selected {
  border-color: var(--primary);
  box-shadow: 0 0 0 1px var(--primary);
}

.kind-glyph {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-sm);
  background: var(--card-2);
  color: var(--primary);
  flex-shrink: 0;
}

.kind-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.kind-label {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.kind-desc {
  font-size: var(--fs-xs);
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.kind-check {
  position: absolute;
  top: 6px;
  right: 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--primary);
  color: #fff;
  font-size: 10px;
  font-weight: 700;
}

/* 折叠头 */
.collapsible-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  height: var(--h-control-md);
  padding: 0 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-2);
  color: var(--text);
  font-family: inherit;
  font-size: var(--fs-sm);
  font-weight: 500;
  text-align: left;
  cursor: pointer;
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
  box-sizing: border-box;
}

.collapsible-head:hover,
.collapsible-head.expanded {
  border-color: var(--primary);
}

.collapsible-arrow {
  color: var(--muted);
  font-size: var(--fs-sm);
}

/* 工具开关行 */
.field-row {
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}

.field-row-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.field-row-hint {
  font-size: var(--fs-xs);
  color: var(--muted);
}

.field-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 保存区 ---------- */
.save-block {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 4px;
  padding: 14px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.save-row {
  display: flex;
  gap: 8px;
}

.save-row .field-input {
  flex: 1;
}

/* ---------- 响应式 ---------- */
@media (max-width: 520px) {
  .field-row-2col {
    grid-template-columns: 1fr;
  }
}
</style>
