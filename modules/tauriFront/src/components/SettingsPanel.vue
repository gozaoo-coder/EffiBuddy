<script setup lang="ts">
import { ref, watch, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { AgentConfig, BackendKind } from '../types'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void; (e: 'saved', backend: string): void }>()

// 本地编辑副本，保存时才回写到后端
const draft = ref<AgentConfig>({
  backend: 'mock',
  api_key: '',
  base_url: '',
  model_name: 'gpt-4o-mini',
  preamble: '你是 EffiSuite 的 AI 助手，简洁友好地回答用户问题。',
  enable_tools: true,
})
const saving = ref(false)
const errorMsg = ref('')
const successMsg = ref('')

// 监听外部热替换（例如其他窗口修改了配置）
let unlisten: UnlistenFn | null = null

onMounted(async () => {
  try {
    draft.value = await invoke<AgentConfig>('get_config')
  } catch (e) {
    errorMsg.value = `加载配置失败：${e}`
  }
  unlisten = await listen('agent-backend-changed', async () => {
    try {
      draft.value = await invoke<AgentConfig>('get_config')
    } catch {
      // 静默
    }
  })
})

watch(
  () => props.open,
  (v) => {
    if (v) {
      errorMsg.value = ''
      successMsg.value = ''
    }
  },
)

function selectBackend(b: BackendKind) {
  draft.value.backend = b
}

async function save() {
  saving.value = true
  errorMsg.value = ''
  successMsg.value = ''
  try {
    await invoke('set_config', { config: draft.value })
    successMsg.value = '已保存并热替换 agent'
    emit('saved', draft.value.backend)
  } catch (e) {
    errorMsg.value = `保存失败：${e}`
  } finally {
    saving.value = false
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

        <!-- OpenAI 配置（仅 openai 模式显示） -->
        <template v-if="draft.backend === 'openai'">
          <div class="field">
            <label class="field-label">API Key</label>
            <input
              v-model="draft.api_key"
              type="password"
              placeholder="sk-..."
              class="field-input"
            />
          </div>

          <div class="field">
            <label class="field-label">Base URL（可选，留空使用默认）</label>
            <input
              v-model="draft.base_url"
              type="text"
              placeholder="https://api.openai.com/v1"
              class="field-input"
            />
          </div>

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

        <!-- Preamble（两种后端都支持） -->
        <div class="field">
          <label class="field-label">系统提示词（preamble）</label>
          <textarea
            v-model="draft.preamble"
            rows="4"
            placeholder="定义 agent 的人设与行为约束"
            class="field-input field-textarea"
          ></textarea>
        </div>

        <!-- 工具开关（仅 openai 模式有意义，但 mock 也保留以便扩展） -->
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

        <!-- 状态提示 -->
        <div v-if="errorMsg" class="msg msg-error">{{ errorMsg }}</div>
        <div v-if="successMsg" class="msg msg-success">{{ successMsg }}</div>
      </div>

      <footer class="settings-foot">
        <button class="btn btn-ghost" @click="onClose">取消</button>
        <button class="btn btn-primary" :disabled="saving" @click="save">
          {{ saving ? '保存中…' : '保存并应用' }}
        </button>
      </footer>
    </div>
  </div>
</template>
