<script setup lang="ts">
import { ref, watch, onMounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { AgentConfig, AvailableModel, ProviderPreset, BackendKind } from '../types'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void; (e: 'saved', backend: string): void }>()

// 当前激活的完整配置（含 models 列表）
const config = ref<AgentConfig | null>(null)
// 内置 provider 预设
const presets = ref<ProviderPreset[]>([])
// draft：当前编辑的运行时字段（保存到 AgentConfig 顶层）
const draft = ref({
  backend: 'mock' as BackendKind,
  provider_id: 'openai',
  api_key: '',
  base_url: '',
  model_name: 'gpt-4o-mini',
  preamble: '你是 EffiSuite 的 AI 助手，简洁友好地回答用户问题。',
  enable_tools: true,
})

const saving = ref(false)
const errorMsg = ref('')
const successMsg = ref('')
// 保存为模型时的标签输入
const saveLabel = ref('')
// 当前展开的 provider 预设 id（高亮）
const activePresetId = computed(() => draft.value.provider_id)

onMounted(async () => {
  try {
    const [c, p] = await Promise.all([
      invoke<AgentConfig>('get_config'),
      invoke<ProviderPreset[]>('list_provider_presets'),
    ])
    config.value = c
    presets.value = p
    syncDraftFromConfig(c)
  } catch (e) {
    errorMsg.value = `加载配置失败：${e}`
  }
})

function syncDraftFromConfig(c: AgentConfig) {
  draft.value = {
    backend: c.backend,
    provider_id: c.provider_id || 'openai',
    api_key: c.api_key,
    base_url: c.base_url,
    model_name: c.model_name,
    preamble: c.preamble,
    enable_tools: c.enable_tools,
  }
  saveLabel.value = ''
}

watch(
  () => props.open,
  (v) => {
    if (v) {
      errorMsg.value = ''
      successMsg.value = ''
      // 重新加载
      invoke<AgentConfig>('get_config')
        .then((c) => {
          config.value = c
          syncDraftFromConfig(c)
        })
        .catch((e) => (errorMsg.value = `加载失败：${e}`))
    }
  },
)

// 选择 provider 预设：填充默认 base_url 和 model_name（仅当用户当前为空或切换时）
function selectPreset(p: ProviderPreset) {
  draft.value.provider_id = p.id
  // 切换预设时自动填充默认值（除非用户已自定义）
  if (p.id !== 'custom') {
    draft.value.base_url = p.default_base_url
    if (p.default_model) draft.value.model_name = p.default_model
  }
  draft.value.backend = 'openai'
}

function selectBackend(b: BackendKind) {
  draft.value.backend = b
}

function findPreset(id: string): ProviderPreset | undefined {
  return presets.value.find((p) => p.id === id)
}

// ---------- 保存当前 draft 到 config（应用为运行时配置） ----------
async function save() {
  if (!config.value) return
  saving.value = true
  errorMsg.value = ''
  successMsg.value = ''
  try {
    const newConfig: AgentConfig = {
      ...config.value,
      backend: draft.value.backend,
      provider_id: draft.value.provider_id,
      api_key: draft.value.api_key,
      base_url: draft.value.base_url,
      model_name: draft.value.model_name,
      preamble: draft.value.preamble,
      enable_tools: draft.value.enable_tools,
    }
    await invoke('set_config', { config: newConfig })
    config.value = newConfig
    successMsg.value = '已保存并热替换 agent'
    emit('saved', draft.value.backend === 'openai' ? 'rig-openai-compat' : 'mock')
  } catch (e) {
    errorMsg.value = `保存失败：${e}`
  } finally {
    saving.value = false
  }
}

// ---------- 保存当前 draft 为可使用模型 ----------
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
    errorMsg.value = '请输入模型标签'
    return
  }
  errorMsg.value = ''
  try {
    const model: AvailableModel = {
      id: newId(),
      label,
      provider_id: draft.value.provider_id,
      base_url: draft.value.base_url,
      model_name: draft.value.model_name,
      api_key: draft.value.api_key,
      preamble: draft.value.preamble,
      enable_tools: draft.value.enable_tools,
      created_at: Date.now(),
    }
    await invoke('save_model', { model })
    saveLabel.value = ''
    successMsg.value = `已保存模型「${label}」`
    // 刷新 config
    config.value = await invoke<AgentConfig>('get_config')
  } catch (e) {
    errorMsg.value = `保存模型失败：${e}`
  }
}

// ---------- 激活某个已保存模型 ----------
async function activateModel(id: string) {
  errorMsg.value = ''
  successMsg.value = ''
  try {
    await invoke('set_active_model', { id })
    config.value = await invoke<AgentConfig>('get_config')
    if (config.value) syncDraftFromConfig(config.value)
    successMsg.value = '已切换模型并热替换 agent'
    emit('saved', 'rig-openai-compat')
  } catch (e) {
    errorMsg.value = `激活失败：${e}`
  }
}

async function deleteModel(id: string) {
  if (!confirm('删除该可使用模型？')) return
  errorMsg.value = ''
  try {
    await invoke('delete_model', { id })
    config.value = await invoke<AgentConfig>('get_config')
    successMsg.value = '已删除'
  } catch (e) {
    errorMsg.value = `删除失败：${e}`
  }
}

function onClose() {
  emit('close')
}
</script>

<template>
  <div v-if="props.open" class="settings-overlay" @click.self="onClose">
    <div class="settings-panel">
      <header class="settings-head">
        <h2>Agent 配置</h2>
        <button class="close-btn" @click="onClose">×</button>
      </header>

      <div class="settings-body">
        <!-- Backend 选择 -->
        <div class="field">
          <label class="field-label">后端类型</label>
          <div class="seg">
            <button
              class="seg-btn"
              :class="{ active: draft.backend === 'mock' }"
              @click="selectBackend('mock')"
            >Mock（离线）</button>
            <button
              class="seg-btn"
              :class="{ active: draft.backend === 'openai' }"
              @click="selectBackend('openai')"
            >OpenAI 兼容</button>
          </div>
        </div>

        <!-- Provider 预设快选（仅 openai 模式显示） -->
        <template v-if="draft.backend === 'openai'">
          <div class="field">
            <label class="field-label">Provider 预设（点击快速填充）</label>
            <div class="preset-grid">
              <button
                v-for="p in presets"
                :key="p.id"
                class="preset-chip"
                :class="{ active: activePresetId === p.id }"
                @click="selectPreset(p)"
              >
                {{ p.name }}
              </button>
            </div>
            <div v-if="findPreset(activePresetId)?.docs_url" class="preset-hint">
              <a :href="findPreset(activePresetId)!.docs_url" target="_blank" rel="noopener">
                文档：{{ findPreset(activePresetId)!.docs_url }}
              </a>
              <span v-if="findPreset(activePresetId)!.env_var">
                · 推荐环境变量 {{ findPreset(activePresetId)!.env_var }}
              </span>
            </div>
          </div>

          <!-- API Key -->
          <div class="field">
            <label class="field-label">API Key</label>
            <input
              v-model="draft.api_key"
              type="password"
              placeholder="sk-..."
              class="field-input"
            />
          </div>

          <!-- Base URL（custom 预设时强调可填任意） -->
          <div class="field">
            <label class="field-label">
              Base URL
              <span v-if="activePresetId === 'custom'" class="hint-tag">自定义</span>
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
            <input
              v-model="draft.model_name"
              type="text"
              placeholder="gpt-4o-mini"
              class="field-input"
            />
          </div>
        </template>

        <!-- Preamble -->
        <div class="field">
          <label class="field-label">系统提示词（preamble）</label>
          <textarea
            v-model="draft.preamble"
            rows="4"
            placeholder="定义 agent 的人设与行为约束"
            class="field-input field-textarea"
          ></textarea>
        </div>

        <!-- 工具开关 -->
        <div class="field field-row">
          <label class="field-label">启用工具调用（RAG 历史检索 / 时间查询）</label>
          <button
            class="toggle"
            :class="{ on: draft.enable_tools }"
            :aria-pressed="draft.enable_tools"
            @click="draft.enable_tools = !draft.enable_tools"
          >
            <span class="toggle-knob"></span>
          </button>
        </div>

        <!-- 保存当前配置 -->
        <div class="actions-row">
          <button class="btn btn-primary" :disabled="saving" @click="save">
            {{ saving ? '保存中…' : '应用为当前配置' }}
          </button>
        </div>

        <!-- 保存为可使用模型 -->
        <div class="field save-as-model">
          <label class="field-label">保存为可使用模型（输入标签后保存）</label>
          <div class="save-row">
            <input
              v-model="saveLabel"
              type="text"
              placeholder="例如：我的 GPT-4o / DeepSeek 工作号"
              class="field-input"
            />
            <button class="btn btn-ghost" @click="saveAsModel">保存</button>
          </div>
        </div>

        <!-- 可使用模型列表 -->
        <div class="field" v-if="config && config.models.length > 0">
          <label class="field-label">可使用模型（点击切换激活）</label>
          <div class="model-list">
            <div
              v-for="m in config.models"
              :key="m.id"
              class="model-item"
              :class="{ active: config.active_model_id === m.id }"
            >
              <div class="model-info" @click="activateModel(m.id)">
                <div class="model-label">{{ m.label }}</div>
                <div class="model-meta">
                  {{ m.provider_id }} · {{ m.model_name }}
                  <span v-if="config.active_model_id === m.id" class="active-tag">当前</span>
                </div>
              </div>
              <button class="model-del" @click.stop="deleteModel(m.id)">×</button>
            </div>
          </div>
        </div>

        <!-- 状态提示 -->
        <div v-if="errorMsg" class="msg msg-error">{{ errorMsg }}</div>
        <div v-if="successMsg" class="msg msg-success">{{ successMsg }}</div>
      </div>

      <footer class="settings-foot">
        <button class="btn btn-ghost" @click="onClose">关闭</button>
      </footer>
    </div>
  </div>
</template>
