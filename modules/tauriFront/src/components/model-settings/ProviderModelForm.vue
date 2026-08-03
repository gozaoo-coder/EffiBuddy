<script setup lang="ts">
/**
 * ProviderModelForm 模型编辑表单（AI 服务商面板的子组件）
 *
 * 职责：
 * - 编辑或新建一个 AvailableModel
 * - 接收 initialModel 作为初始 draft（编辑模式）或空 draft（新建模式，kind 由 folder 决定）
 * - 字段：服务商、API Key、Base URL、模型名、上下文窗口、计费、图像参数、工具开关、preamble、标签
 * - 提交时 emit save(model) / cancel()
 *
 * 不持有业务状态：所有数据本地化在 draft，emit 出去由父组件持久化。
 * 表单字段较多但属于"单模型编辑"的不可拆分单元，独立成子组件避免父面板膨胀。
 */
import { ref, watch, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  Button,
  IconButton,
  Icon,
  Switch,
  Dropdown,
  type DropdownOption,
  useToast,
} from '../basic'
import type {
  AvailableModel,
  ModelKind,
  ProviderPreset,
  RemoteModelInfo,
  ModelPricing,
} from '../../types'

const props = defineProps<{
  /** 初始模型（编辑模式）或 null（新建模式） */
  initial: AvailableModel | null
  /** 锁定的模型类型（来自 folder 选择）；新建时强制使用此 kind，编辑时跟随 initial */
  lockedKind: ModelKind
  /** 内置 provider 预设列表 */
  presets: ProviderPreset[]
  /** 是否编辑模式（initial 非空时为 true） */
  isEditing: boolean
}>()

const emit = defineEmits<{
  (e: 'save', model: AvailableModel): void
  (e: 'cancel'): void
}>()

const { toast } = useToast()

// ---------- Draft ----------
interface Draft {
  provider_id: string
  api_key: string
  base_url: string
  model_name: string
  preamble: string
  enable_tools: boolean
  kind: ModelKind
  image_size: string
  image_quality: string
  video_resolution: string
  video_ratio: string
  /** number 输入框经 v-model 自动转为 number，清空时回退为空字符串 */
  video_duration: string | number
  audio_language: string
  context_window_tokens: number
  pricing: { cache_hit_per_m: number | null; cache_miss_per_m: number | null; output_per_m: number | null }
  label: string
  id: string
}

const DEFAULT_PREAMBLE =
  '你是 EffiSuite 的 AI 助手。遵守以下准则：\n【回答】简短直接，不重复用户问题，不堆砌铺垫与废话。\n【执行】调用工具前，先用 1-3 句话简述最优实现路径（含关键步骤/文件/技术选型），再发起工具调用。\n【原则】优先最短路径，避免试错；工具失败时给出明确下一步，不空谈。'

function emptyDraft(kind: ModelKind): Draft {
  return {
    id: '',
    provider_id: '',
    api_key: '',
    base_url: '',
    model_name: '',
    preamble: DEFAULT_PREAMBLE,
    enable_tools: true,
    kind,
    image_size: '',
    image_quality: '',
    video_resolution: '',
    video_ratio: '',
    video_duration: '',
    audio_language: '',
    context_window_tokens: 128000,
    pricing: { cache_hit_per_m: null, cache_miss_per_m: null, output_per_m: null },
    label: '',
  }
}

function draftFromModel(m: AvailableModel): Draft {
  return {
    id: m.id,
    provider_id: m.provider_id,
    api_key: m.api_key,
    base_url: m.base_url,
    model_name: m.model_name,
    preamble: m.preamble,
    enable_tools: m.enable_tools,
    kind: m.kind ?? 'chat',
    image_size: m.image_size ?? '',
    image_quality: m.image_quality ?? '',
    video_resolution: m.video_resolution ?? '',
    video_ratio: m.video_ratio ?? '',
    video_duration: m.video_duration != null ? String(m.video_duration) : '',
    audio_language: m.audio_language ?? '',
    context_window_tokens: m.context_window_tokens ?? 128000,
    pricing: m.pricing
      ? { ...m.pricing }
      : { cache_hit_per_m: null, cache_miss_per_m: null, output_per_m: null },
    label: m.label,
  }
}

const draft = ref<Draft>(
  props.initial ? draftFromModel(props.initial) : emptyDraft(props.lockedKind),
)

// 当父组件传入新的 initial（切换编辑目标）或 lockedKind（切换 folder）时重置 draft
watch(
  () => props.initial,
  (m) => {
    draft.value = m ? draftFromModel(m) : emptyDraft(props.lockedKind)
    remoteModels.value = []
    showApiKey.value = false
    preambleExpanded.value = !!m
  },
)
watch(
  () => props.lockedKind,
  (k) => {
    // 仅在新建模式下跟随 folder 变化；编辑模式保持 initial 的 kind
    if (!props.isEditing) {
      draft.value.kind = k
    }
  },
)

const showApiKey = ref(false)
const preambleExpanded = ref(!!props.initial)
const saving = ref(false)

// ---------- Kind 视觉与选项 ----------
const kindMeta: Record<ModelKind, { label: string; icon: string; desc: string }> = {
  chat: { label: '对话模型', icon: 'chat', desc: 'LLM 文本对话与推理' },
  image_gen: { label: '图像生成', icon: 'image', desc: 'DALL-E / SD / Flux 等文生图' },
  video_gen: { label: '视频生成', icon: 'camera', desc: '文生视频（预留）' },
  audio_transcribe: { label: '音频转文字', icon: 'mic', desc: 'Whisper 等音频转写' },
}

const currentKindMeta = computed(() => kindMeta[draft.value.kind])
const isImageGen = computed(() => draft.value.kind === 'image_gen')
const isVideoGen = computed(() => draft.value.kind === 'video_gen')
const isAudioTranscribe = computed(() => draft.value.kind === 'audio_transcribe')
const isChat = computed(() => draft.value.kind === 'chat')

// ---------- Provider 视觉映射 ----------
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

function findPreset(id: string): ProviderPreset | undefined {
  return props.presets.find((p) => p.id === id)
}

// 推荐模型列表（部分 provider，仅对话模型使用）
const recommendedModels: Record<string, string[]> = {
  openai: ['gpt-4o-mini', 'gpt-4o', 'gpt-4-turbo', 'gpt-3.5-turbo'],
  deepseek: ['deepseek-chat', 'deepseek-reasoner'],
  groq: ['llama-3.3-70b-versatile', 'llama-3.1-8b-instant', 'mixtral-8x7b-32768'],
  anthropic: ['claude-3-5-sonnet-20241022', 'claude-3-5-haiku-20241022'],
  moonshot: ['moonshot-v1-8k', 'moonshot-v1-32k', 'moonshot-v1-128k'],
}

// ---------- 从 API 拉取可用模型 ----------
const remoteModels = ref<RemoteModelInfo[]>([])
const fetchingModels = ref(false)

const modelDropdownOptions = computed<DropdownOption[]>(() => {
  if (remoteModels.value.length > 0) {
    return remoteModels.value.map((m) => ({
      label: m.id + (m.owned_by ? ` · ${m.owned_by}` : ''),
      value: m.id,
    }))
  }
  const list = recommendedModels[draft.value.provider_id]
  if (!list) return []
  return list.map((m) => ({ label: m, value: m }))
})

const hasModelRecommendations = computed(() => modelDropdownOptions.value.length > 0)

async function fetchRemoteModels() {
    if (!draft.value.base_url.trim()) {
      toast({ content: '请先填写 Base URL', type: 'warn' })
      return
    }
    fetchingModels.value = true
    try {
      const list = await invoke<RemoteModelInfo[]>('list_remote_models', {
        baseUrl: draft.value.base_url.trim(),
        apiKey: draft.value.api_key.trim(),
      })
      remoteModels.value = list
      if (list.length === 0) {
        toast({ content: 'API 返回空列表（服务未返回可用模型）', type: 'warn' })
      } else {
        toast({ content: `已获取 ${list.length} 个可用模型`, type: 'success' })
      }
    } catch (e) {
      toast({ content: `拉取模型失败：${errText(e)}`, type: 'error' })
      remoteModels.value = []
    } finally {
      fetchingModels.value = false
    }
  }

  /** 把 invoke 抛出的未知错误转成可读字符串（可能是 string / Error / 对象） */
  function errText(e: unknown): string {
    if (typeof e === 'string') return e
    if (e instanceof Error) return e.message
    try {
      return JSON.stringify(e)
    } catch {
      return String(e)
    }
  }

function onModelNameDropdownChange(v: string | number, _opt: DropdownOption) {
  draft.value.model_name = String(v)
}

// ---------- Provider 选择 ----------
function selectPreset(p: ProviderPreset) {
  draft.value.provider_id = p.id
  if (p.id !== 'custom') {
    draft.value.base_url = p.default_base_url
    if (p.default_model) draft.value.model_name = p.default_model
  }
  remoteModels.value = []
}

function selectCustom() {
  draft.value.provider_id = 'custom'
  remoteModels.value = []
}

// ---------- 图像尺寸/质量预设 ----------
const imageSizePresets = ['1024x1024', '1792x1024', '1024x1792', '512x512', '256x256']
const imageQualityPresets = ['standard', 'hd']

// ---------- 视频生成预设（与 generate_video 工具参数对齐） ----------
const videoResolutionPresets = ['480p', '720p']
const videoRatioPresets = ['16:9', '4:3', '1:1', '3:4', '9:16', '21:9', 'adaptive']

// ---------- 音频转文字预设 ----------
const audioLanguagePresets = ['auto', 'zh', 'en', 'ja', 'ko', 'ru', 'fr', 'de', 'es']

// ---------- 保存 ----------
function newId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID()
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

function save() {
  const label = draft.value.label.trim()
  if (!label) {
    toast({ content: '请输入模型标签', type: 'warn' })
    return
  }
  if (!draft.value.api_key && draft.value.provider_id !== 'custom') {
    toast({ content: '请填写 API Key', type: 'warn' })
    return
  }
  saving.value = true
  // 计费：任一单价 > 0 时保存为 ModelPricing，否则 null
  const pricingVals = {
    cache_hit_per_m: Number(draft.value.pricing.cache_hit_per_m || 0),
    cache_miss_per_m: Number(draft.value.pricing.cache_miss_per_m || 0),
    output_per_m: Number(draft.value.pricing.output_per_m || 0),
  }
  const hasPricing =
    pricingVals.cache_hit_per_m > 0 ||
    pricingVals.cache_miss_per_m > 0 ||
    pricingVals.output_per_m > 0
  const pricing: ModelPricing | null = hasPricing ? pricingVals : null

  const model: AvailableModel = {
    id: draft.value.id || newId(),
    label,
    provider_id: draft.value.provider_id,
    base_url: draft.value.base_url,
    model_name: draft.value.model_name,
    api_key: draft.value.api_key,
    preamble: draft.value.preamble,
    enable_tools: draft.value.enable_tools,
    kind: draft.value.kind,
    image_size:
      draft.value.kind === 'image_gen' && draft.value.image_size.trim()
        ? draft.value.image_size.trim()
        : null,
    image_quality:
      draft.value.kind === 'image_gen' && draft.value.image_quality.trim()
        ? draft.value.image_quality.trim()
        : null,
    video_resolution:
      draft.value.kind === 'video_gen' && draft.value.video_resolution.trim()
        ? draft.value.video_resolution.trim()
        : null,
    video_ratio:
      draft.value.kind === 'video_gen' && draft.value.video_ratio.trim()
        ? draft.value.video_ratio.trim()
        : null,
    video_duration:
      draft.value.kind === 'video_gen' &&
      String(draft.value.video_duration).trim() !== ''
        ? Math.max(2, Math.min(15, Number(draft.value.video_duration) || 0))
        : null,
    audio_language:
      draft.value.kind === 'audio_transcribe' && draft.value.audio_language.trim()
        ? draft.value.audio_language.trim()
        : null,
    context_window_tokens: draft.value.context_window_tokens || null,
    pricing,
    created_at: Date.now(),
  }
  emit('save', model)
  saving.value = false
}

function cancel() {
  emit('cancel')
}
</script>

<template>
  <div class="pmf">
    <!-- 表单顶部：返回 + 标题 + 类型徽标 -->
    <header class="pmf-head">
      <button type="button" class="pmf-back" @click="cancel">
        <Icon name="chevron-right" :size="14" class="pmf-back-icon" />
        返回列表
      </button>
      <h3 class="pmf-title">
        {{ isEditing ? `编辑：${draft.label || '未命名模型'}` : '添加新模型' }}
      </h3>
      <span class="pmf-kind-badge" :class="`pmf-kind-badge--${draft.kind}`">
        <Icon :name="currentKindMeta.icon" :size="12" />
        {{ currentKindMeta.label }}
      </span>
      <span v-if="isEditing" class="pmf-editing-tag">编辑中</span>
    </header>

    <div class="pmf-body">
      <!-- 服务商选择 -->
      <div class="pmf-field">
        <label class="pmf-label">服务商</label>
        <div class="pmf-provider-grid">
          <button
            v-for="p in presets"
            :key="p.id"
            type="button"
            class="pmf-provider-card"
            :class="{ selected: draft.provider_id === p.id }"
            @click="selectPreset(p)"
          >
            <span
              class="pmf-provider-glyph"
              :style="{ background: visualOf(p.id).accent }"
            ><Icon :name="visualOf(p.id).glyph" :size="16" /></span>
            <span class="pmf-provider-name">{{ p.name }}</span>
            <span v-if="draft.provider_id === p.id" class="pmf-check">✓</span>
          </button>
          <button
            v-if="!presets.some((p) => p.id === 'custom')"
            type="button"
            class="pmf-provider-card"
            :class="{ selected: draft.provider_id === 'custom' }"
            @click="selectCustom"
          >
            <span class="pmf-provider-glyph" :style="{ background: visualOf('custom').accent }">
              <Icon :name="visualOf('custom').glyph" :size="16" />
            </span>
            <span class="pmf-provider-name">自定义</span>
            <span v-if="draft.provider_id === 'custom'" class="pmf-check">✓</span>
          </button>
        </div>
        <p v-if="findPreset(draft.provider_id)?.docs_url" class="pmf-provider-hint">
          <a :href="findPreset(draft.provider_id)!.docs_url" target="_blank" rel="noopener">
            📖 {{ findPreset(draft.provider_id)!.docs_url }}
          </a>
          <span v-if="findPreset(draft.provider_id)!.env_var" class="pmf-env-tag">
            推荐 {{ findPreset(draft.provider_id)!.env_var }}
          </span>
        </p>
      </div>

      <!-- API Key -->
      <div class="pmf-field">
        <label class="pmf-label">API Key</label>
        <div class="pmf-input-with-action">
          <input
            v-model="draft.api_key"
            :type="showApiKey ? 'text' : 'password'"
            placeholder="sk-..."
            class="pmf-input"
          />
          <IconButton
            size="sm"
            container
            :title="showApiKey ? '隐藏' : '显示'"
            @click="showApiKey = !showApiKey"
          >
            <Icon :name="showApiKey ? 'eye-off' : 'eye'" :size="16" />
          </IconButton>
        </div>
      </div>

      <!-- Base URL + Model Name -->
      <div class="pmf-row-2col">
        <div class="pmf-field">
          <label class="pmf-label">
            Base URL
            <span v-if="draft.provider_id === 'custom'" class="pmf-hint-tag">自定义</span>
          </label>
          <input
            v-model="draft.base_url"
            type="text"
            placeholder="https://api.openai.com/v1"
            class="pmf-input"
          />
        </div>
        <div class="pmf-field">
          <label class="pmf-label">
            模型名
            <span
              v-if="remoteModels.length > 0"
              class="pmf-api-badge"
              :title="`来自 API（${remoteModels.length} 个可用模型）`"
            >API</span>
          </label>
          <div class="pmf-model-row">
            <Dropdown
              v-if="hasModelRecommendations && isChat"
              :model-value="draft.model_name"
              :options="modelDropdownOptions"
              :searchable="true"
              :placeholder="remoteModels.length > 0 ? '搜索 API 模型...' : '选择或搜索推荐模型...'"
              size="md"
              class="pmf-model-dropdown"
              @change="onModelNameDropdownChange"
            />
            <input
              v-else
              v-model="draft.model_name"
              type="text"
              :placeholder="isImageGen ? 'dall-e-3 / flux-pro / sd3' : 'gpt-4o-mini'"
              class="pmf-input"
            />
            <button
              type="button"
              class="pmf-fetch-btn"
              :disabled="fetchingModels || !isChat"
              :title="!isChat ? '仅对话模型支持拉取列表' : '从 API 拉取可用模型'"
              @click="fetchRemoteModels"
            >
              <span v-if="fetchingModels" class="pmf-spinner"></span>
              <Icon v-else name="refresh" :size="14" />
              <span>{{ fetchingModels ? '拉取中' : '从 API 获取' }}</span>
            </button>
          </div>
        </div>
      </div>

      <!-- 上下文窗口（仅 chat） -->
      <div v-if="isChat" class="pmf-field">
        <label class="pmf-label">上下文窗口（tokens）</label>
        <input
          v-model.number="draft.context_window_tokens"
          type="number"
          min="1024"
          step="1024"
          placeholder="128000"
          class="pmf-input"
        />
        <p class="pmf-field-hint">用于估算当前对话已用上下文比例</p>
      </div>

      <!-- 计费单价（仅 chat） -->
      <div v-if="isChat" class="pmf-field">
        <label class="pmf-label">计费单价（元 / 百万 tokens）</label>
        <div class="pmf-row-3col">
          <div class="pmf-field">
            <label class="pmf-label">缓存命中输入</label>
            <input
              v-model.number="draft.pricing.cache_hit_per_m"
              type="number"
              min="0"
              step="0.001"
              placeholder="如 0.02"
              class="pmf-input"
            />
          </div>
          <div class="pmf-field">
            <label class="pmf-label">缓存未命中输入</label>
            <input
              v-model.number="draft.pricing.cache_miss_per_m"
              type="number"
              min="0"
              step="0.001"
              placeholder="如 1"
              class="pmf-input"
            />
          </div>
          <div class="pmf-field">
            <label class="pmf-label">输出</label>
            <input
              v-model.number="draft.pricing.output_per_m"
              type="number"
              min="0"
              step="0.001"
              placeholder="如 2"
              class="pmf-input"
            />
          </div>
        </div>
        <p class="pmf-field-hint">
          对应 DeepSeek 等 provider 的计费规则（输入分缓存命中/未命中，输出单独计费）；
          全部留空则不显示消费金额
        </p>
      </div>

      <!-- 图像生成专用字段 -->
      <template v-if="isImageGen">
        <div class="pmf-row-2col">
          <div class="pmf-field">
            <label class="pmf-label">默认尺寸</label>
            <input
              v-model="draft.image_size"
              type="text"
              placeholder="1024x1024（留空用默认）"
              class="pmf-input"
              list="pmf-size-list"
            />
            <datalist id="pmf-size-list">
              <option v-for="s in imageSizePresets" :key="s" :value="s" />
            </datalist>
          </div>
          <div class="pmf-field">
            <label class="pmf-label">默认质量</label>
            <input
              v-model="draft.image_quality"
              type="text"
              placeholder="standard / hd"
              class="pmf-input"
              list="pmf-quality-list"
            />
            <datalist id="pmf-quality-list">
              <option v-for="q in imageQualityPresets" :key="q" :value="q" />
            </datalist>
          </div>
        </div>
      </template>

      <!-- 视频生成专用字段 -->
      <template v-if="isVideoGen">
        <div class="pmf-row-2col">
          <div class="pmf-field">
            <label class="pmf-label">默认分辨率</label>
            <input
              v-model="draft.video_resolution"
              type="text"
              placeholder="720p（留空用模型默认）"
              class="pmf-input"
              list="pmf-video-res-list"
            />
            <datalist id="pmf-video-res-list">
              <option v-for="r in videoResolutionPresets" :key="r" :value="r" />
            </datalist>
          </div>
          <div class="pmf-field">
            <label class="pmf-label">默认宽高比</label>
            <input
              v-model="draft.video_ratio"
              type="text"
              placeholder="16:9（留空用模型默认）"
              class="pmf-input"
              list="pmf-video-ratio-list"
            />
            <datalist id="pmf-video-ratio-list">
              <option v-for="r in videoRatioPresets" :key="r" :value="r" />
            </datalist>
          </div>
        </div>
        <div class="pmf-field">
          <label class="pmf-label">默认时长（秒）</label>
          <input
            v-model="draft.video_duration"
            type="number"
            min="2"
            max="15"
            step="1"
            placeholder="留空用模型默认（2 ~ 15 秒）"
            class="pmf-input"
          />
          <p class="pmf-field-hint">作为 generate_video 工具调用时的默认值，越界输入会自动收敛到 2 ~ 15</p>
        </div>
      </template>

      <!-- 音频转文字专用字段 -->
      <template v-if="isAudioTranscribe">
        <div class="pmf-field">
          <label class="pmf-label">默认源语言</label>
          <input
            v-model="draft.audio_language"
            type="text"
            placeholder="auto / zh / en（留空用后端默认）"
            class="pmf-input"
            list="pmf-audio-lang-list"
          />
          <datalist id="pmf-audio-lang-list">
            <option v-for="l in audioLanguagePresets" :key="l" :value="l" />
          </datalist>
          <p class="pmf-field-hint">转写时使用的源语言；auto 表示自动检测</p>
        </div>
      </template>

      <!-- 启用工具（仅 chat） -->
      <div v-if="isChat" class="pmf-field pmf-field-row">
        <div class="pmf-row-text">
          <label class="pmf-label">启用工具调用</label>
          <span class="pmf-row-hint">RAG 历史检索 / 时间查询 / 图像生成</span>
        </div>
        <Switch v-model="draft.enable_tools" size="md" />
      </div>

      <!-- Preamble（折叠） -->
      <div class="pmf-field">
        <button
          type="button"
          class="pmf-collapsible"
          :class="{ expanded: preambleExpanded }"
          @click="preambleExpanded = !preambleExpanded"
        >
          <span>系统提示词（preamble）</span>
          <Icon
            :name="preambleExpanded ? 'chevron-down' : 'chevron-right'"
            :size="14"
          />
        </button>
        <textarea
          v-if="preambleExpanded"
          v-model="draft.preamble"
          rows="4"
          placeholder="定义 agent 的人设与行为约束"
          class="pmf-input pmf-textarea"
        ></textarea>
      </div>

      <!-- 保存区 -->
      <div class="pmf-save-block">
        <label class="pmf-label">模型标签</label>
        <div class="pmf-save-row">
          <input
            v-model="draft.label"
            type="text"
            :placeholder="isEditing ? '修改标签名' : '例如：我的 GPT-4o 工作号'"
            class="pmf-input"
          />
          <Button variant="primary" :loading="saving" @click="save">
            {{ isEditing ? '更新并返回' : '保存并返回' }}
          </Button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.pmf {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

.pmf-head {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 20px;
  border-bottom: 1px solid var(--border);
  background: var(--bg);
  flex-shrink: 0;
}

.pmf-back {
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

.pmf-back:hover {
  border-color: var(--primary);
  color: var(--primary);
}

.pmf-back-icon {
  transform: rotate(180deg);
}

.pmf-title {
  flex: 1;
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pmf-kind-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  font-size: var(--fs-xs);
  border-radius: var(--radius-full);
  font-weight: 500;
  flex-shrink: 0;
}

.pmf-kind-badge--chat {
  color: var(--primary);
  background: rgba(74, 126, 255, 0.12);
}

.pmf-kind-badge--image_gen {
  color: #a855f7;
  background: rgba(168, 85, 247, 0.12);
}

.pmf-kind-badge--video_gen {
  color: #f59e0b;
  background: rgba(245, 158, 11, 0.12);
}

.pmf-kind-badge--audio_transcribe {
  color: #10b981;
  background: rgba(16, 185, 129, 0.12);
}

.pmf-editing-tag {
  flex-shrink: 0;
  padding: 2px 8px;
  font-size: var(--fs-xs);
  color: var(--warn);
  background: rgba(240, 192, 74, 0.14);
  border: 1px solid rgba(240, 192, 74, 0.4);
  border-radius: var(--radius-full);
}

.pmf-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 16px 20px 28px;
  overflow-y: auto;
}

/* ---------- 字段 ---------- */
.pmf-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.pmf-label {
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text);
}

.pmf-input {
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

.pmf-input:focus {
  border-color: var(--primary);
}

.pmf-textarea {
  height: auto;
  min-height: 84px;
  padding: 10px 12px;
  resize: vertical;
  line-height: 1.5;
  font-family: inherit;
}

.pmf-row-2col {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}

.pmf-row-3col {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 12px;
}

.pmf-input-with-action {
  display: flex;
  gap: 8px;
}

.pmf-input-with-action .pmf-input {
  flex: 1;
}

.pmf-hint-tag {
  display: inline-block;
  margin-left: 6px;
  padding: 1px 6px;
  font-size: var(--fs-xs);
  color: var(--warn);
  background: rgba(240, 192, 74, 0.12);
  border-radius: var(--radius-xs);
}

.pmf-api-badge {
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

.pmf-field-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 服务商网格 ---------- */
.pmf-provider-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  gap: 8px;
}

.pmf-provider-card {
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
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

.pmf-provider-card:hover {
  border-color: var(--primary);
  background: var(--card-2);
}

.pmf-provider-card.selected {
  border-color: var(--primary);
  box-shadow: 0 0 0 1px var(--primary);
}

.pmf-provider-glyph {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: var(--radius-sm);
  color: #fff;
  flex-shrink: 0;
}

.pmf-provider-name {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.pmf-check {
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
  flex-shrink: 0;
}

.pmf-provider-hint {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-xs);
  color: var(--muted);
  margin: 0;
}

.pmf-provider-hint a {
  color: var(--primary);
  text-decoration: none;
}

.pmf-provider-hint a:hover {
  text-decoration: underline;
}

.pmf-env-tag {
  padding: 1px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  font-family: 'SFMono-Regular', Consolas, monospace;
}

/* ---------- 模型名行 ---------- */
.pmf-model-row {
  display: flex;
  gap: 8px;
  align-items: stretch;
}

.pmf-model-dropdown {
  flex: 1;
  min-width: 0;
}

.pmf-model-row .pmf-input {
  flex: 1;
  min-width: 0;
}

.pmf-fetch-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--card);
  color: var(--text);
  cursor: pointer;
  font-size: var(--fs-xs);
  font-weight: 500;
  white-space: nowrap;
  font-family: inherit;
  transition: all var(--duration-fast) var(--ease-standard);
}

.pmf-fetch-btn:hover:not(:disabled) {
  border-color: var(--primary);
  color: var(--primary);
}

.pmf-fetch-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.pmf-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid currentColor;
  border-top-color: transparent;
  border-radius: 50%;
  animation: pmf-spin 0.8s linear infinite;
}

@keyframes pmf-spin {
  to {
    transform: rotate(360deg);
  }
}

/* ---------- 折叠头 ---------- */
.pmf-collapsible {
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

.pmf-collapsible:hover,
.pmf-collapsible.expanded {
  border-color: var(--primary);
}

/* ---------- 工具开关行 ---------- */
.pmf-field-row {
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}

.pmf-row-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.pmf-row-hint {
  font-size: var(--fs-xs);
  color: var(--muted);
}

/* ---------- 保存区 ---------- */
.pmf-save-block {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 4px;
  padding: 14px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.pmf-save-row {
  display: flex;
  gap: 8px;
}

.pmf-save-row .pmf-input {
  flex: 1;
}

/* ---------- 响应式 ---------- */
@media (max-width: 560px) {
  .pmf-row-2col,
  .pmf-row-3col {
    grid-template-columns: 1fr;
  }
}
</style>
