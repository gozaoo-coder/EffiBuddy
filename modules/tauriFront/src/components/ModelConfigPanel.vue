<script setup lang="ts">
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
  type DropdownOption,
  type MenuItemOption,
  useToast,
  useSnackbar,
} from './basic'
import { useAnimeTransition } from '../composables/useAnimeTransition'
import type { AgentConfig, AvailableModel, ProviderPreset, BackendKind, ModelKind } from '../types'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void; (e: 'saved', backend: string): void }>()

const { toast } = useToast()
const { snackbar } = useSnackbar()

// 完整配置（含 models 列表与 active_model_id）
const config = ref<AgentConfig | null>(null)
// 内置 provider 预设
const presets = ref<ProviderPreset[]>([])

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
})

// 模型类型选项
const kindOptions: { value: ModelKind; label: string; icon: string; desc: string }[] = [
  { value: 'chat', label: '对话模型', icon: 'chat', desc: 'LLM 文本对话与推理' },
  { value: 'image_gen', label: '图像生成', icon: 'image', desc: 'DALL-E / SD / Flux 等文生图' },
]

// 图像尺寸预设
const imageSizePresets = ['1024x1024', '1792x1024', '1024x1792', '512x512', '256x256']
const imageQualityPresets = ['standard', 'hd']

const isImageGen = computed(() => draft.value.kind === 'image_gen')

// 是否已选择 provider（控制 Step 2 展开）
const providerSelected = computed(() => !!draft.value.provider_id)
// 当前编辑的模型 id（非空表示编辑模式）
const editingId = ref<string | null>(null)

// 保存为模型时的标签
const saveLabel = ref('')
const saving = ref(false)

// API Key 显隐
const showApiKey = ref(false)
// Preamble 折叠
const preambleExpanded = ref(false)

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

const modelDropdownOptions = computed<DropdownOption[]>(() => {
  const list = recommendedModels[draft.value.provider_id]
  if (!list) return []
  return list.map((m) => ({ label: m, value: m }))
})

const hasModelRecommendations = computed(() => modelDropdownOptions.value.length > 0)

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
    syncDraftFromConfig(c)
  } catch (e) {
    toast({ content: `加载配置失败：${e}`, type: 'error' })
  }
}

function syncDraftFromConfig(c: AgentConfig) {
  draft.value = {
    backend: c.backend,
    provider_id: c.provider_id || '',
    api_key: c.api_key,
    base_url: c.base_url,
    model_name: c.model_name,
    preamble: c.preamble,
    enable_tools: c.enable_tools,
    kind: 'chat' as ModelKind,
    image_size: '',
    image_quality: '',
  }
  editingId.value = null
  saveLabel.value = ''
  showApiKey.value = false
  preambleExpanded.value = false
}

watch(
  () => props.open,
  (v) => {
    if (v) {
      invoke<AgentConfig>('get_config')
        .then((c) => {
          config.value = c
          syncDraftFromConfig(c)
        })
        .catch((e) => toast({ content: `加载失败：${e}`, type: 'error' }))
    }
  },
)

function findPreset(id: string): ProviderPreset | undefined {
  return presets.value.find((p) => p.id === id)
}

// ---------- Step 1: 选择 provider ----------
function selectPreset(p: ProviderPreset) {
  draft.value.provider_id = p.id
  if (p.id !== 'custom') {
    draft.value.base_url = p.default_base_url
    if (p.default_model) draft.value.model_name = p.default_model
  }
  draft.value.backend = 'openai'
  editingId.value = null
}

function resetSelection() {
  draft.value.provider_id = ''
  draft.value.api_key = ''
  draft.value.base_url = ''
  draft.value.model_name = ''
  editingId.value = null
  saveLabel.value = ''
}

// ---------- Step 2 ↔ Step 1 切换动画 ----------
const { onEnter: onStepEnter, onLeave: onStepLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    translateY: [16, 0],
    duration: 340,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    translateY: [0, -8],
    duration: 200,
    ease: 'inOut(2)',
  },
})

function onModelNameDropdownChange(v: string | number, _opt: DropdownOption) {
  draft.value.model_name = String(v)
}

// ---------- Step 3: 保存为可用模型 ----------
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
      image_size: draft.value.kind === 'image_gen' && draft.value.image_size.trim()
        ? draft.value.image_size.trim()
        : null,
      image_quality: draft.value.kind === 'image_gen' && draft.value.image_quality.trim()
        ? draft.value.image_quality.trim()
        : null,
      created_at: Date.now(),
    }
    await invoke('save_model', { model })
    saveLabel.value = ''
    editingId.value = null
    toast({
      content: wasEditing ? `已更新模型「${label}」` : `已保存模型「${label}」`,
      type: 'success',
    })
    config.value = await invoke<AgentConfig>('get_config')
  } catch (e) {
    toast({ content: `保存模型失败：${e}`, type: 'error' })
  } finally {
    saving.value = false
  }
}

// 编辑已有模型：载入 draft
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
  }
  editingId.value = m.id
  saveLabel.value = m.label
  preambleExpanded.value = true
  toast({ content: `正在编辑「${m.label}」`, type: 'info' })
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
      if (config.value) syncDraftFromConfig(config.value)
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
  const isActive = !!m && config.value?.active_model_id === m.id
  return [
    { key: 'edit', label: '编辑', icon: 'edit' },
    { key: 'default', label: isActive ? '当前默认' : '设为默认', icon: 'star', divided: true, disabled: isActive },
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

// ---------- 空状态与统计 ----------
const hasModels = computed(() => !!config.value && config.value.models.length > 0)
const activeModel = computed(() =>
  config.value?.models.find((m) => m.id === config.value?.active_model_id) ?? null,
)

function onClose() {
  emit('close')
}
</script>

<template>
  <BindSheet
    :visible="props.open"
    side="right"
    width="560px"
    title="可用模型配置"
    @close="onClose"
  >
    <div class="mcp-body">
      <!-- 顶部欢迎语 -->
      <header class="mcp-hero">
        <div class="hero-mark">🤖</div>
        <div class="hero-text">
          <h2 class="hero-title">配置你的 AI 模型</h2>
          <p class="hero-sub">选择服务商，填入凭据，即可保存为可切换的可用模型</p>
        </div>
      </header>

      <!-- Step 1: Provider 选择 -->
      <section class="step">
        <div class="step-head">
          <span class="step-index">1</span>
          <span class="step-title">选择服务商</span>
          <button
            v-if="providerSelected"
            type="button"
            class="step-reset"
            @click="resetSelection"
          >
            重新选择
          </button>
        </div>

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
            ><Icon :name="visualOf(p.id).glyph" :size="20" /></span>
            <span class="provider-info">
              <span class="provider-name">{{ p.name }}</span>
              <span class="provider-desc">{{ visualOf(p.id).desc }}</span>
            </span>
            <span v-if="draft.provider_id === p.id" class="provider-check">✓</span>
          </button>
          <!-- 若 presets 不含 custom，补充自定义卡片 -->
          <button
            v-if="!presets.some((p) => p.id === 'custom')"
            type="button"
            class="provider-card"
            :class="{ selected: draft.provider_id === 'custom' }"
            @click="selectPreset({ id: 'custom', name: '自定义', default_base_url: '', default_model: '', env_var: '', docs_url: '', openai_compat: true } as ProviderPreset)"
          >
            <span class="provider-glyph" :style="{ background: visualOf('custom').accent }"><Icon :name="visualOf('custom').glyph" :size="20" /></span>
            <span class="provider-info">
              <span class="provider-name">自定义</span>
              <span class="provider-desc">{{ visualOf('custom').desc }}</span>
            </span>
            <span v-if="draft.provider_id === 'custom'" class="provider-check">✓</span>
          </button>
        </div>

        <div v-if="findPreset(draft.provider_id)?.docs_url" class="provider-hint">
          <a :href="findPreset(draft.provider_id)!.docs_url" target="_blank" rel="noopener">
            📖 文档：{{ findPreset(draft.provider_id)!.docs_url }}
          </a>
          <span v-if="findPreset(draft.provider_id)!.env_var" class="env-tag">
            推荐环境变量 {{ findPreset(draft.provider_id)!.env_var }}
          </span>
        </div>
      </section>

      <!-- Step 2: 模型信息填写（选中 provider 后展开） -->
      <Transition :css="false" @enter="onStepEnter" @leave="onStepLeave">
        <section v-if="providerSelected" class="step step--form">
          <div class="step-head">
            <span class="step-index">2</span>
            <span class="step-title">填写模型信息</span>
            <span v-if="editingId" class="editing-tag">编辑中</span>
          </div>

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

          <!-- Base URL -->
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

          <!-- Model Name -->
          <div class="field">
            <label class="field-label">模型名</label>
            <Dropdown
              v-if="hasModelRecommendations && !isImageGen"
              :model-value="draft.model_name"
              :options="modelDropdownOptions"
              :searchable="true"
              placeholder="选择或搜索推荐模型..."
              size="md"
              @change="onModelNameDropdownChange"
            />
            <input
              v-else
              v-model="draft.model_name"
              type="text"
              :placeholder="isImageGen ? 'dall-e-3 / flux-pro / sd3' : 'gpt-4o-mini'"
              class="field-input"
            />
          </div>

          <!-- 图像生成专用字段：尺寸与质量（仅 kind=image_gen 时显示） -->
          <template v-if="isImageGen">
            <div class="field">
              <label class="field-label">默认尺寸</label>
              <input
                v-model="draft.image_size"
                type="text"
                placeholder="1024x1024（留空用模型默认）"
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
                placeholder="standard / hd（留空用模型默认）"
                class="field-input"
                list="image-quality-list"
              />
              <datalist id="image-quality-list">
                <option v-for="q in imageQualityPresets" :key="q" :value="q" />
              </datalist>
            </div>
          </template>

          <!-- System Preamble（折叠） -->
          <div class="field">
            <button
              type="button"
              class="collapsible-head"
              :class="{ expanded: preambleExpanded }"
              @click="preambleExpanded = !preambleExpanded"
            >
              <span class="collapsible-label">系统提示词（preamble）</span>
              <span class="collapsible-arrow"><Icon :name="preambleExpanded ? 'chevron-down' : 'chevron-right'" :size="14" /></span>
            </button>
            <textarea
              v-if="preambleExpanded"
              v-model="draft.preamble"
              rows="4"
              placeholder="定义 agent 的人设与行为约束"
              class="field-input field-textarea"
            ></textarea>
          </div>

          <!-- Enable Tools（仅对话模型有意义） -->
          <div v-if="!isImageGen" class="field field-row">
            <div class="field-row-text">
              <label class="field-label">启用工具调用</label>
              <span class="field-row-hint">RAG 历史检索 / 时间查询 / 图像生成</span>
            </div>
            <Switch v-model="draft.enable_tools" size="md" />
          </div>
        </section>
      </Transition>

      <!-- Step 3: 保存为可用模型 -->
      <section v-if="providerSelected" class="step">
        <div class="step-head">
          <span class="step-index">3</span>
          <span class="step-title">保存与激活</span>
        </div>

        <div class="save-block">
          <label class="field-label">模型标签</label>
          <div class="save-row">
            <input
              v-model="saveLabel"
              type="text"
              :placeholder="editingId ? '修改标签名' : '例如：我的 GPT-4o 工作号'"
              class="field-input"
            />
            <Button variant="primary" :loading="saving" @click="saveAsModel">
              {{ editingId ? '更新' : '保存' }}
            </Button>
          </div>
        </div>
      </section>

      <!-- 已保存模型列表 -->
      <section class="step">
        <div class="step-head">
          <span class="step-title list-title">已保存模型</span>
          <span v-if="hasModels" class="count-badge">{{ config!.models.length }}</span>
        </div>

        <!-- 空状态引导 -->
        <div v-if="!hasModels" class="empty-state">
          <div class="empty-illust">✦</div>
          <p class="empty-text">还没有可用模型</p>
          <p class="empty-hint">在上方选择服务商并填写信息，配置你的第一个模型吧</p>
        </div>

        <!-- 模型卡片列表 -->
        <div v-else class="model-list">
          <div
            v-for="m in config!.models"
            :key="m.id"
            class="model-card"
            :class="{
              active: config!.active_model_id === m.id || config!.active_image_gen_model_id === m.id,
            }"
          >
            <div class="model-card-main" @click="activateModel(m.id)">
              <span
                class="model-card-glyph"
                :style="{ background: visualOf(m.provider_id).accent }"
              ><Icon :name="(m.kind ?? 'chat') === 'image_gen' ? 'image' : visualOf(m.provider_id).glyph" :size="20" /></span>
              <div class="model-card-info">
                <div class="model-card-top">
                  <span class="model-card-label">{{ m.label }}</span>
                  <span
                    v-if="m.kind === 'image_gen'"
                    class="kind-pill kind-pill--image"
                  >图像</span>
                  <span
                    v-else
                    class="kind-pill kind-pill--chat"
                  >对话</span>
                  <span
                    v-if="config!.active_model_id === m.id"
                    class="active-pill"
                  >对话已激活</span>
                  <span
                    v-else-if="config!.active_image_gen_model_id === m.id"
                    class="active-pill active-pill--image"
                  >图像已激活</span>
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
              <Icon name="more-horizontal" :size="20" />
            </IconButton>
          </div>
        </div>
      </section>
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
  gap: 20px;
  overflow-y: auto;
}

/* ---------- 顶部欢迎语 ---------- */
.mcp-hero {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 18px 20px;
  background: linear-gradient(135deg, rgba(74, 126, 255, 0.14), rgba(74, 126, 255, 0.02));
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

/* ---------- 通用步骤 ---------- */
.step {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.step--form {
  padding: 18px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.step-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

.step-index {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: var(--primary);
  color: #fff;
  font-size: var(--fs-sm);
  font-weight: 600;
  flex-shrink: 0;
}

.step-title {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.list-title {
  margin-left: 0;
}

.step-reset {
  margin-left: auto;
  padding: 2px 10px;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--primary);
  background: transparent;
  border: 1px solid var(--primary);
  border-radius: var(--radius-full);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
}

.step-reset:hover {
  background: var(--primary);
  color: #fff;
}

.editing-tag {
  margin-left: auto;
  padding: 2px 8px;
  font-size: var(--fs-xs);
  color: var(--warn);
  background: rgba(240, 192, 74, 0.14);
  border: 1px solid rgba(240, 192, 74, 0.4);
  border-radius: var(--radius-full);
}

.count-badge {
  margin-left: auto;
  padding: 2px 10px;
  font-size: var(--fs-xs);
  color: var(--muted);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
}

/* ---------- Provider 卡片网格 ---------- */
.provider-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}

.provider-card {
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
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
  transform: scale(1.01);
}

.provider-glyph {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-sm);
  color: #fff;
  font-size: 18px;
  flex-shrink: 0;
}

.provider-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.provider-name {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.provider-desc {
  font-size: var(--fs-xs);
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.provider-check {
  position: absolute;
  top: 8px;
  right: 8px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--primary);
  color: #fff;
  font-size: 11px;
  font-weight: 700;
}

.provider-hint {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-xs);
  color: var(--muted);
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

/* ---------- 保存区 ---------- */
.save-block {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.save-row {
  display: flex;
  gap: 8px;
}

.save-row .field-input {
  flex: 1;
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

/* ---------- 模型卡片列表 ---------- */
.model-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
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

.model-card-meta {
  font-size: var(--fs-xs);
  color: var(--muted);
  margin-top: 3px;
  font-family: 'SFMono-Regular', Consolas, monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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

/* 模型卡片类型标签 */
.kind-pill {
  flex-shrink: 0;
  padding: 1px 7px;
  font-size: var(--fs-xs);
  border-radius: var(--radius-full);
  font-weight: 500;
}

.kind-pill--chat {
  color: var(--primary);
  background: rgba(74, 126, 255, 0.12);
}

.kind-pill--image {
  color: #a855f7;
  background: rgba(168, 85, 247, 0.12);
}

.active-pill--image {
  background: #a855f7 !important;
}
</style>
