<script setup lang="ts">
import { ref, watch, onMounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  BindSheet,
  Button,
  IconButton,
  Switch,
  SegmentedButton,
  Chips,
  Dropdown,
  type DropdownOption,
  type SegmentedOption,
  useToast,
  useSnackbar,
} from './basic'
import type { AgentConfig, AvailableModel, ProviderPreset, BackendKind } from '../types'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void; (e: 'saved', backend: string): void }>()

const { toast } = useToast()
const { snackbar } = useSnackbar()

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
// 保存为模型时的标签输入
const saveLabel = ref('')

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
    toast({ content: `加载配置失败：${e}`, type: 'error' })
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
      // 重新加载
      invoke<AgentConfig>('get_config')
        .then((c) => {
          config.value = c
          syncDraftFromConfig(c)
        })
        .catch((e) => toast({ content: `加载失败：${e}`, type: 'error' }))
    }
  },
)

// Backend 选项：用 SegmentedButton
const backendOptions: SegmentedOption[] = [
  { label: 'Mock（离线）', value: 'mock' },
  { label: 'OpenAI 兼容', value: 'openai' },
]

function onBackendChange(v: string | number) {
  draft.value.backend = v as BackendKind
}

// Provider 预设：作为 Dropdown 选项（也可作 Chips 快选）
const presetDropdownOptions = computed<DropdownOption[]>(() =>
  presets.value.map((p) => ({
    label: p.name,
    value: p.id,
    icon: '◆',
  })),
)

function onPresetDropdownChange(_v: string | number, opt: DropdownOption) {
  const p = presets.value.find((x) => x.id === opt.value)
  if (!p) return
  selectPreset(p)
}

// 选择 provider 预设：填充默认 base_url 和 model_name
function selectPreset(p: ProviderPreset) {
  draft.value.provider_id = p.id
  if (p.id !== 'custom') {
    draft.value.base_url = p.default_base_url
    if (p.default_model) draft.value.model_name = p.default_model
  }
  draft.value.backend = 'openai'
}

function findPreset(id: string): ProviderPreset | undefined {
  return presets.value.find((p) => p.id === id)
}

// ---------- 保存当前 draft 到 config（应用为运行时配置） ----------
async function save() {
  if (!config.value) return
  saving.value = true
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
    toast({ content: '已保存并热替换 agent', type: 'success' })
    emit('saved', draft.value.backend === 'openai' ? 'rig-openai-compat' : 'mock')
  } catch (e) {
    toast({ content: `保存失败：${e}`, type: 'error' })
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
    toast({ content: '请输入模型标签', type: 'warn' })
    return
  }
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
    toast({ content: `已保存模型「${label}」`, type: 'success' })
    // 刷新 config
    config.value = await invoke<AgentConfig>('get_config')
  } catch (e) {
    toast({ content: `保存模型失败：${e}`, type: 'error' })
  }
}

// ---------- 激活某个已保存模型 ----------
async function activateModel(id: string) {
  try {
    await invoke('set_active_model', { id })
    config.value = await invoke<AgentConfig>('get_config')
    if (config.value) syncDraftFromConfig(config.value)
    toast({ content: '已切换模型并热替换 agent', type: 'success' })
    emit('saved', 'rig-openai-compat')
  } catch (e) {
    toast({ content: `激活失败：${e}`, type: 'error' })
  }
}

// 用 snackbar 带撤销操作确认删除
async function deleteModel(id: string, label: string) {
  // 先记录当前 config 以便撤销
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
          // 简单撤销：把模型重新保存回去
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

function onClose() {
  emit('close')
}
</script>

<template>
  <BindSheet
    :visible="props.open"
    side="right"
    width="480px"
    title="Agent 配置"
    @close="onClose"
  >
    <div class="settings-body">
      <!-- Backend 选择 -->
      <div class="field">
        <label class="field-label">后端类型</label>
        <SegmentedButton
          :model-value="draft.backend"
          :options="backendOptions"
          block
          size="md"
          @change="onBackendChange"
        />
      </div>

      <!-- Provider 预设快选（仅 openai 模式显示） -->
      <template v-if="draft.backend === 'openai'">
        <div class="field">
          <label class="field-label">Provider 预设</label>
          <Dropdown
            :model-value="draft.provider_id"
            :options="presetDropdownOptions"
            placeholder="选择 provider..."
            size="md"
            @change="onPresetDropdownChange"
          />
          <!-- 预设 chips 快选 -->
          <div class="preset-chips-row">
            <Chips
              v-for="p in presets"
              :key="p.id"
              :label="p.name"
              :selected="draft.provider_id === p.id"
              size="sm"
              @click="selectPreset(p)"
            />
          </div>
          <div v-if="findPreset(draft.provider_id)?.docs_url" class="preset-hint">
            <a :href="findPreset(draft.provider_id)!.docs_url" target="_blank" rel="noopener">
              文档：{{ findPreset(draft.provider_id)!.docs_url }}
            </a>
            <span v-if="findPreset(draft.provider_id)!.env_var">
              · 推荐环境变量 {{ findPreset(draft.provider_id)!.env_var }}
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
        <div class="field-row-text">
          <label class="field-label">启用工具调用</label>
          <span class="field-row-hint">RAG 历史检索 / 时间查询</span>
        </div>
        <Switch v-model="draft.enable_tools" size="md" />
      </div>

      <!-- 保存当前配置 -->
      <div class="actions-row">
        <Button variant="primary" block :loading="saving" @click="save">
          {{ saving ? '保存中…' : '应用为当前配置' }}
        </Button>
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
          <Button variant="normal" @click="saveAsModel">保存</Button>
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
            <IconButton
              icon="✕"
              size="sm"
              variant="danger"
              title="删除该模型"
              @click="deleteModel(m.id, m.label)"
            />
          </div>
        </div>
      </div>
    </div>

    <!-- 底部关闭 -->
    <div class="settings-foot">
      <Button variant="text" block @click="onClose">关闭</Button>
    </div>
  </BindSheet>
</template>

<style scoped>
.settings-body {
  padding: 16px 20px;
  display: flex;
  flex-direction: column;
  gap: 18px;
  overflow-y: auto;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.field-label {
  font-size: 13px;
  font-weight: 500;
  color: var(--text);
}

.field-input {
  width: 100%;
  height: var(--h-control-md);
  padding: 0 12px;
  font-family: inherit;
  font-size: 14px;
  color: var(--text);
  background: var(--card);
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
  min-height: 88px;
  padding: 10px 12px;
  resize: vertical;
  line-height: 1.5;
  font-family: inherit;
}

.preset-chips-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 4px;
}

.preset-hint {
  font-size: 12px;
  color: var(--muted);
  margin-top: 4px;
  line-height: 1.6;
}

.preset-hint a {
  color: var(--primary);
  text-decoration: none;
}

.preset-hint a:hover {
  text-decoration: underline;
}

.hint-tag {
  display: inline-block;
  margin-left: 6px;
  padding: 1px 6px;
  font-size: 11px;
  color: var(--primary);
  border: 1px solid var(--primary);
  border-radius: var(--radius-xs);
}

.field-row {
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}

.field-row-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.field-row-hint {
  font-size: 11px;
  color: var(--muted);
}

.actions-row {
  margin-top: 4px;
}

.save-row {
  display: flex;
  gap: 8px;
}

.save-row .field-input {
  flex: 1;
}

.model-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.model-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  transition: all var(--duration-fast) var(--ease-standard);
}

.model-item:hover {
  border-color: var(--primary);
}

.model-item.active {
  border-color: var(--primary);
  background: rgba(74, 126, 255, 0.08);
}

.model-info {
  flex: 1;
  min-width: 0;
  cursor: pointer;
}

.model-label {
  font-size: 14px;
  font-weight: 500;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.model-meta {
  font-size: 12px;
  color: var(--muted);
  margin-top: 2px;
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.active-tag {
  display: inline-block;
  margin-left: 6px;
  padding: 1px 6px;
  font-size: 10px;
  color: #fff;
  background: var(--primary);
  border-radius: var(--radius-xs);
}

.settings-foot {
  padding: 12px 20px;
  border-top: 1px solid var(--border);
}
</style>
