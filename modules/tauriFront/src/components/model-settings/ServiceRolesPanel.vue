<script setup lang="ts">
/**
 * ServiceRolesPanel 服务模型面板
 *
 * 配置各使用场景的默认模型：按场景分组（聊天 / 语音 / 生图）展示角色槽位。
 *
 * 角色与 AgentConfig 字段映射：
 * - 聊天组：
 *   - chat（聊天模型） → active_model_id
 *   - title（对话命名模型） → title_model_id
 *   - compression（会话历史压缩模型） → compression_model_id
 * - 语音组：
 *   - asr_stream（语音实时转文字） → asr_stream_model_id
 *   - asr_transcribe（音频转文字） → asr_transcribe_model_id
 * - 生图组：
 *   - image_gen（默认生图模型） → active_image_gen_model_id
 *
 * 数据流：
 * - onMounted 加载 config
 * - 监听 agent-backend-changed：外部修改后刷新
 * - 选择/清除模型通过 set_service_model_role 命令，刷新 config 后视图同步
 * - emit saved 通知父组件刷新后端信息
 *
 * 槽位视觉：
 * - 卡片：左 = 角色信息（图标 + 标题 + 描述），右 = Dropdown 当前模型 + 清除按钮
 * - Dropdown 选项按 allowedKinds 过滤 config.models
 * - 未配置时 Dropdown 显示"未配置"占位
 * - chat 角色 = 主对话模型，重建 agent，菜单中标注"主对话"
 */
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  Icon,
  IconButton,
  Dropdown,
  type DropdownOption,
  useToast,
} from '../basic'
import type {
  AgentConfig,
  AvailableModel,
  ModelKind,
  ServiceModelRole,
  ServiceRoleMeta,
} from '../../types'

const emit = defineEmits<{ (e: 'saved'): void }>()
const { toast } = useToast()

// ---------- 角色元数据（按场景分组） ----------
const roleMetas: ServiceRoleMeta[] = [
  // 聊天组
  {
    role: 'chat',
    group: 'chat',
    label: '聊天模型',
    desc: '主对话 agent，所有用户消息由它处理',
    allowedKinds: ['chat'],
    configField: 'active_model_id',
  },
  {
    role: 'title',
    group: 'chat',
    label: '对话命名模型',
    desc: '为新会话自动生成标题（auto_classify）',
    allowedKinds: ['chat'],
    configField: 'title_model_id',
  },
  {
    role: 'compression',
    group: 'chat',
    label: '会话历史压缩模型',
    desc: '长会话压缩决策，建议选擅长长文本的高性价比模型',
    allowedKinds: ['chat'],
    configField: 'compression_model_id',
  },
  // 语音组
  {
    role: 'asr_stream',
    group: 'voice',
    label: '语音实时转文字模型',
    desc: '流式录音转写，建议低延迟模型（如 whisper-1）',
    allowedKinds: ['audio_transcribe'],
    configField: 'asr_stream_model_id',
  },
  {
    role: 'asr_transcribe',
    group: 'voice',
    label: '音频转文字模型',
    desc: '音频文件批量转写',
    allowedKinds: ['audio_transcribe'],
    configField: 'asr_transcribe_model_id',
  },
  // 生图组
  {
    role: 'image_gen',
    group: 'image',
    label: '默认生图模型',
    desc: 'LLM 调用 image_gen 工具时使用',
    allowedKinds: ['image_gen'],
    configField: 'active_image_gen_model_id',
  },
]

// 分组元数据
const groupMetas: { key: 'chat' | 'voice' | 'image'; label: string; icon: string; desc: string; accent: string }[] = [
  { key: 'chat', label: '聊天', icon: 'chat', desc: '文本对话与历史管理', accent: '#4a7eff' },
  { key: 'voice', label: '语音', icon: 'mic', desc: '语音转写相关', accent: '#10b981' },
  { key: 'image', label: '生图', icon: 'image', desc: '图像生成', accent: '#a855f7' },
]

// ---------- 数据 ----------
const config = ref<AgentConfig | null>(null)
const savingRole = ref<ServiceModelRole | null>(null)

let unlistenBackend: UnlistenFn | null = null
onMounted(async () => {
  await loadConfig()
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

async function loadConfig() {
  try {
    config.value = await invoke<AgentConfig>('get_config')
  } catch (e) {
    toast({ content: `加载配置失败：${e}`, type: 'error' })
  }
}

// ---------- 角色当前模型 id ----------
function currentModelIdOf(role: ServiceModelRole): string | null {
  if (!config.value) return null
  switch (role) {
    case 'chat':
      return config.value.active_model_id
    case 'image_gen':
      return config.value.active_image_gen_model_id ?? null
    case 'title':
      return config.value.title_model_id ?? null
    case 'compression':
      return config.value.compression_model_id ?? null
    case 'asr_stream':
      return config.value.asr_stream_model_id ?? null
    case 'asr_transcribe':
      return config.value.asr_transcribe_model_id ?? null
  }
}

function findModel(id: string | null): AvailableModel | null {
  if (!id || !config.value) return null
  return config.value.models.find((m) => m.id === id) ?? null
}

// ---------- 角色可选模型列表（按 allowedKinds 过滤） ----------
function optionsForRole(meta: ServiceRoleMeta): DropdownOption[] {
  if (!config.value) return []
  return config.value.models
    .filter((m) => meta.allowedKinds.includes(m.kind ?? 'chat'))
    .map((m) => ({
      label: `${m.label} · ${m.model_name}`,
      value: m.id,
      icon: m.provider_id,
    }))
}

// ---------- 选择/清除 ----------
async function onRoleChange(meta: ServiceRoleMeta, value: string | number, _opt: DropdownOption) {
  const modelId = String(value)
  savingRole.value = meta.role
  try {
    await invoke('set_service_model_role', {
      role: meta.role,
      modelId,
    })
    config.value = await invoke<AgentConfig>('get_config')
    const m = findModel(modelId)
    toast({
      content: `已设置${meta.label}：${m?.label ?? modelId}`,
      type: 'success',
    })
    emit('saved')
  } catch (e) {
    toast({ content: `设置失败：${e}`, type: 'error' })
  } finally {
    savingRole.value = null
  }
}

async function clearRole(meta: ServiceRoleMeta) {
  savingRole.value = meta.role
  try {
    await invoke('set_service_model_role', {
      role: meta.role,
      modelId: null,
    })
    config.value = await invoke<AgentConfig>('get_config')
    toast({ content: `已清除${meta.label}`, type: 'success' })
    emit('saved')
  } catch (e) {
    toast({ content: `清除失败：${e}`, type: 'error' })
  } finally {
    savingRole.value = null
  }
}

// ---------- 视觉 ----------
const roleGlyph: Record<ServiceModelRole, string> = {
  chat: 'chat',
  title: 'edit',
  compression: 'merge',
  asr_stream: 'mic',
  asr_transcribe: 'mic',
  image_gen: 'image',
}

const roleAccent: Record<ServiceModelRole, string> = {
  chat: '#4a7eff',
  title: '#6c5ce7',
  compression: '#0ea5e9',
  asr_stream: '#10b981',
  asr_transcribe: '#059669',
  image_gen: '#a855f7',
}
</script>

<template>
  <section class="srp">
    <!-- 顶部说明 -->
    <header class="srp-head">
      <div class="srp-head-text">
        <h2 class="srp-title">
          <Icon name="robot" :size="20" />
          服务模型
        </h2>
        <p class="srp-desc">
          为各使用场景配置默认模型。可选模型来自"AI 服务商"中已添加的同类型模型。
        </p>
      </div>
    </header>

    <div v-if="!config" class="srp-loading">
      <Icon name="loader" :size="20" />
      加载中...
    </div>

    <div v-else class="srp-body">
      <!-- 按场景分组渲染 -->
      <section
        v-for="g in groupMetas"
        :key="g.key"
        class="srp-group"
      >
        <header class="srp-group-head">
          <span class="srp-group-glyph" :style="{ background: g.accent }">
            <Icon :name="g.icon" :size="16" />
          </span>
          <div class="srp-group-text">
            <h3 class="srp-group-label">{{ g.label }}</h3>
            <p class="srp-group-desc">{{ g.desc }}</p>
          </div>
        </header>

        <div class="srp-slots">
          <div
            v-for="meta in roleMetas.filter((r) => r.group === g.key)"
            :key="meta.role"
            class="srp-slot"
            :class="{ 'srp-slot--active': !!currentModelIdOf(meta.role) }"
          >
            <div class="srp-slot-left">
              <span
                class="srp-slot-glyph"
                :style="{ background: roleAccent[meta.role] }"
              >
                <Icon :name="roleGlyph[meta.role]" :size="16" />
              </span>
              <div class="srp-slot-info">
                <div class="srp-slot-title">
                  {{ meta.label }}
                  <span
                    v-if="meta.role === 'chat'"
                    class="srp-slot-tag srp-slot-tag--primary"
                  >主对话</span>
                </div>
                <p class="srp-slot-desc">{{ meta.desc }}</p>
              </div>
            </div>

            <div class="srp-slot-right">
              <Dropdown
                :model-value="currentModelIdOf(meta.role) ?? ''"
                :options="optionsForRole(meta)"
                :searchable="true"
                :placeholder="optionsForRole(meta).length === 0 ? `无可用${meta.allowedKinds.join('/')}模型` : '选择模型...'"
                :disabled="optionsForRole(meta).length === 0 || savingRole === meta.role"
                size="md"
                class="srp-slot-dropdown"
                @change="(v, opt) => onRoleChange(meta, v, opt)"
              />
              <IconButton
                v-if="currentModelIdOf(meta.role)"
                size="sm"
                container
                title="清除配置"
                :disabled="savingRole === meta.role"
                @click="clearRole(meta)"
              >
                <Icon name="close" :size="14" />
              </IconButton>
            </div>
          </div>
        </div>
      </section>

      <!-- 底部提示 -->
      <footer class="srp-foot">
        <p class="srp-foot-hint">
          <Icon name="info" :size="12" />
          未配置的角色会回退到主对话模型或后端默认值；可在"AI 服务商"中添加新模型。
        </p>
      </footer>
    </div>
  </section>
</template>

<style scoped>
.srp {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
  background: var(--bg);
}

/* ---------- 顶部 ---------- */
.srp-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 20px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.srp-head-text {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.srp-title {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 700;
  color: var(--text);
  letter-spacing: 0.2px;
}

.srp-desc {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 加载态 ---------- */
.srp-loading {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--muted);
  font-size: var(--fs-sm);
}

.srp-loading :deep(.app-icon) {
  animation: srp-spin 1s linear infinite;
}

@keyframes srp-spin {
  to {
    transform: rotate(360deg);
  }
}

/* ---------- 主体 ---------- */
.srp-body {
  flex: 1;
  overflow-y: auto;
  padding: 16px 20px 28px;
  display: flex;
  flex-direction: column;
  gap: 20px;
}

/* ---------- 分组 ---------- */
.srp-group {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.srp-group-head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 4px;
}

.srp-group-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border-radius: var(--radius-sm);
  color: #fff;
  flex-shrink: 0;
}

.srp-group-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.srp-group-label {
  margin: 0;
  font-size: var(--fs-sm);
  font-weight: 700;
  color: var(--text);
  letter-spacing: 0.2px;
}

.srp-group-desc {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
}

/* ---------- 角色槽位 ---------- */
.srp-slots {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.srp-slot {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--card);
  transition: border-color var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard);
}

.srp-slot:hover {
  border-color: var(--primary);
}

.srp-slot--active {
  border-color: rgba(74, 126, 255, 0.4);
  background: rgba(74, 126, 255, 0.04);
}

.srp-slot-left {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: 1;
  min-width: 0;
}

.srp-slot-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-sm);
  color: #fff;
  flex-shrink: 0;
}

.srp-slot-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.srp-slot-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.srp-slot-tag {
  display: inline-flex;
  align-items: center;
  padding: 0 6px;
  font-size: 10px;
  font-weight: 500;
  border-radius: var(--radius-xs);
  letter-spacing: 0.3px;
}

.srp-slot-tag--primary {
  color: #fff;
  background: var(--primary);
}

.srp-slot-desc {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.4;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.srp-slot-right {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
  min-width: 220px;
}

.srp-slot-dropdown {
  flex: 1;
  min-width: 180px;
}

/* ---------- 底部提示 ---------- */
.srp-foot {
  padding: 10px 14px;
  border: 1px dashed var(--border);
  border-radius: var(--radius);
  background: var(--card);
}

.srp-foot-hint {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 响应式 ---------- */
@media (max-width: 720px) {
  .srp-head,
  .srp-body {
    padding-left: 14px;
    padding-right: 14px;
  }
  .srp-slot {
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
  }
  .srp-slot-right {
    min-width: 0;
    width: 100%;
  }
}
</style>
