<script setup lang="ts">
/**
 * AgentDefView 建立智能体
 * 列出自定义智能体定义，支持新建 / 编辑 / 删除，选择头像、角色、系统提示词与模型。
 * 纯内容视图，由外部自动化容器承载。
 */
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  Button,
  Dialog,
  IconButton,
  Icon,
  Switch,
  Dropdown,
  useToast,
  type DropdownOption,
} from '../basic'
import type { AgentDef, AgentConfig, AvailableModel } from '../../types'

const { toast } = useToast()

// ---------- 头像候选 ----------
const AVATARS = ['🤖', '🔍', '🛠️', '📝', '🎨', '🧠', '💡', '🔬', '📊', '✍️']

// ---------- 数据 ----------
const defs = ref<AgentDef[]>([])
const loading = ref(false)
const chatModels = ref<AvailableModel[]>([])

// 模型 id → label 映射，用于列表展示
const modelNameMap = computed(() => {
  const m = new Map<string, string>()
  for (const mod of chatModels.value) m.set(mod.id, mod.label)
  return m
})

function modelNameOf(id: string | null | undefined): string {
  if (!id) return '默认模型'
  return modelNameMap.value.get(id) ?? id
}

async function refresh() {
  loading.value = true
  try {
    const [d, cfg] = await Promise.all([
      invoke<AgentDef[]>('list_agent_defs'),
      invoke<AgentConfig>('get_config'),
    ])
    defs.value = d
    chatModels.value = cfg.models.filter((m) => (m.kind ?? 'chat') === 'chat')
  } catch (e) {
    toast({ content: `加载智能体失败：${e}`, type: 'error' })
    defs.value = []
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  refresh()
})

// ---------- 新建 / 编辑 Dialog ----------
const dialogOpen = ref(false)
const editingId = ref<string | null>(null)
const draft = ref({
  name: '',
  role: '',
  avatar: AVATARS[0],
  system_prompt: '',
  model_id: '' as string,
  enable_tools: true,
})

const editTitle = computed(() => (editingId.value ? '编辑智能体' : '新建智能体'))

const modelDropdownOptions = computed<DropdownOption[]>(() => {
  const opts: DropdownOption[] = [{ label: '全局默认', value: '' }]
  for (const m of chatModels.value) {
    opts.push({ label: m.label, value: m.id })
  }
  return opts
})

function onModelChange(v: string | number, _opt: DropdownOption) {
  draft.value.model_id = String(v)
}

function openCreate() {
  editingId.value = null
  draft.value = {
    name: '',
    role: '',
    avatar: AVATARS[0],
    system_prompt: '',
    model_id: '',
    enable_tools: true,
  }
  dialogOpen.value = true
}

function openEdit(def: AgentDef) {
  editingId.value = def.id
  draft.value = {
    name: def.name,
    role: def.role,
    avatar: def.avatar || AVATARS[0],
    system_prompt: def.system_prompt,
    model_id: def.model_id ?? '',
    enable_tools: def.enable_tools,
  }
  dialogOpen.value = true
}

async function confirmSave() {
  const name = draft.value.name.trim()
  if (!name) {
    toast({ content: '请输入智能体名称', type: 'warn' })
    return
  }
  if (!draft.value.role.trim()) {
    toast({ content: '请输入角色描述', type: 'warn' })
    return
  }
  const def: AgentDef = {
    id: editingId.value ?? '',
    name,
    role: draft.value.role.trim(),
    system_prompt: draft.value.system_prompt,
    avatar: draft.value.avatar,
    model_id: draft.value.model_id || null,
    enable_tools: draft.value.enable_tools,
    created_at: 0,
    updated_at: 0,
  }
  try {
    await invoke<AgentDef>('save_agent_def', { def })
    toast({ content: editingId.value ? `已更新「${name}」` : `已创建「${name}」`, type: 'success' })
    dialogOpen.value = false
    await refresh()
  } catch (e) {
    toast({ content: `保存失败：${e}`, type: 'error' })
  }
}

// ---------- 启停工具开关 ----------
async function onToggleTools(def: AgentDef, enabled: boolean) {
  const prev = def.enable_tools
  def.enable_tools = enabled
  try {
    await invoke<AgentDef>('save_agent_def', {
      def: { ...def, enable_tools: enabled },
    })
    toast({ content: enabled ? `已启用「${def.name}」的工具` : `已停用「${def.name}」的工具`, type: 'success' })
  } catch (e) {
    def.enable_tools = prev
    toast({ content: `操作失败：${e}`, type: 'error' })
  }
}

// ---------- 删除 ----------
const deleteDialogOpen = ref(false)
const deleteTarget = ref<AgentDef | null>(null)

function askDelete(def: AgentDef) {
  deleteTarget.value = def
  deleteDialogOpen.value = true
}

async function confirmDelete() {
  const t = deleteTarget.value
  if (!t) return
  try {
    await invoke('delete_agent_def', { id: t.id })
    toast({ content: `已删除智能体「${t.name}」`, type: 'success' })
    await refresh()
  } catch (e) {
    toast({ content: `删除失败：${e}`, type: 'error' })
  } finally {
    deleteTarget.value = null
  }
}
</script>

<template>
  <div class="agent-body">
    <!-- 顶部说明 -->
    <header class="agent-hero">
      <div class="hero-mark"><Icon name="avatar" :size="28" :fallback="'🤖'" /></div>
      <div class="hero-text">
        <h2 class="hero-title">建立智能体</h2>
        <p class="hero-sub">定义角色、系统提示词与模型，主/子 agent 可随时召唤自定义智能体</p>
      </div>
      <div class="hero-action">
        <Button variant="primary" size="sm" @click="openCreate">
          <template #icon><Icon name="plus" :size="16" /></template>
          新建智能体
        </Button>
      </div>
    </header>

    <!-- 智能体列表 -->
    <section class="section">
      <div class="section-head">
        <span class="section-title">智能体列表</span>
        <span v-if="defs.length" class="count-badge">{{ defs.length }}</span>
      </div>

      <div v-if="!defs.length && !loading" class="empty-state">
        <div class="empty-illust"><Icon name="avatar" :size="48" :fallback="'🤖'" /></div>
        <p class="empty-text">还没有智能体</p>
        <p class="empty-hint">点击「新建智能体」，定义第一个专属智能体吧</p>
      </div>

      <div v-else class="agent-list">
        <div v-for="def in defs" :key="def.id" class="agent-card">
          <div class="agent-avatar">{{ def.avatar || '🤖' }}</div>
          <div class="agent-main">
            <div class="agent-top">
              <span class="agent-name">{{ def.name }}</span>
              <span class="agent-model"><Icon name="cpu" :size="14" :fallback="'·'" /> {{ modelNameOf(def.model_id) }}</span>
            </div>
            <p class="agent-role">{{ def.role }}</p>
          </div>
          <div class="agent-actions">
            <Switch
              :model-value="def.enable_tools"
              size="sm"
              title="启用工具"
              @update:model-value="(v: boolean) => onToggleTools(def, v)"
            />
            <IconButton size="sm" title="编辑" @click="openEdit(def)">
              <Icon name="edit-02" :size="18" :fallback="'✎'" />
            </IconButton>
            <IconButton size="sm" title="删除" @click="askDelete(def)">
              <Icon name="delete" :size="18" />
            </IconButton>
          </div>
        </div>
      </div>
    </section>

    <!-- 新建 / 编辑 Dialog -->
    <Dialog
      v-model:visible="dialogOpen"
      :title="editTitle"
      confirm-text="保存"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmSave"
    >
      <div class="form-body">
        <div class="field">
          <label class="field-label">头像</label>
          <div class="avatar-picker">
            <button
              v-for="a in AVATARS"
              :key="a"
              type="button"
              class="avatar-opt"
              :class="{ active: draft.avatar === a }"
              @click="draft.avatar = a"
            >{{ a }}</button>
          </div>
        </div>
        <div class="field">
          <label class="field-label">名称</label>
          <input v-model="draft.name" type="text" class="field-input" placeholder="例如：数据研究员" />
        </div>
        <div class="field">
          <label class="field-label">角色描述</label>
          <input v-model="draft.role" type="text" class="field-input" placeholder="一句话描述这个智能体的职责（如：擅长数据整理与分析）" />
        </div>
        <div class="field">
          <label class="field-label">系统提示词</label>
          <textarea
            v-model="draft.system_prompt"
            class="field-textarea"
            rows="5"
            placeholder="定义该智能体的行为准则、能力边界与输出风格..."
          />
        </div>
        <div class="field">
          <label class="field-label">使用模型</label>
          <Dropdown
            :model-value="draft.model_id"
            :options="modelDropdownOptions"
            :searchable="true"
            placeholder="选择模型..."
            size="md"
            @change="onModelChange"
          />
          <p class="field-hint">选择「全局默认」将使用当前配置的默认对话模型</p>
        </div>
        <div class="field">
          <div class="switch-row">
            <div class="switch-label">
              <span class="field-label">启用工具</span>
              <span class="field-hint">允许该智能体调用工具（搜索、文件、代码等）</span>
            </div>
            <Switch v-model="draft.enable_tools" size="sm" />
          </div>
        </div>
      </div>
    </Dialog>

    <!-- 删除确认 -->
    <Dialog
      v-model:visible="deleteDialogOpen"
      title="删除智能体"
      danger
      confirm-text="删除"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmDelete"
    >
      <div class="dialog-delete-content">
        确定删除智能体「{{ deleteTarget?.name }}」？此操作不可撤销。
      </div>
    </Dialog>
  </div>
</template>

<style scoped>
.agent-body {
  padding: 20px 24px 32px;
  display: flex;
  flex-direction: column;
  gap: 20px;
  overflow-y: auto;
}

/* ---------- 顶部说明 ---------- */
.agent-hero {
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
  border-radius: var(--radius-lg);
  background: var(--card-2);
  font-size: 22px;
  flex-shrink: 0;
}

.hero-text {
  flex: 1;
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

.hero-action {
  flex-shrink: 0;
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

/* ---------- 智能体卡片 ---------- */
.agent-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.agent-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--card);
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.agent-card:hover {
  border-color: var(--primary);
}

.agent-avatar {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border-radius: var(--radius-lg);
  background: var(--card-2);
  font-size: 20px;
  flex-shrink: 0;
}

.agent-main {
  flex: 1;
  min-width: 0;
}

.agent-top {
  display: flex;
  align-items: center;
  gap: 8px;
}

.agent-name {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.agent-model {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 1px 8px;
  font-size: var(--fs-xs);
  color: var(--primary);
  background: rgba(74, 126, 255, 0.1);
  border-radius: var(--radius-full);
}

.agent-role {
  margin: 4px 0 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.agent-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

/* ---------- 空状态 ---------- */
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 36px 20px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
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

/* ---------- Dialog 表单 ---------- */
.form-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 4px 0;
}

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

.field-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
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
  border-radius: var(--radius-md);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.field-input:focus {
  border-color: var(--primary);
}

.field-textarea {
  width: 100%;
  padding: 10px 12px;
  font-family: inherit;
  font-size: var(--fs-base);
  line-height: 1.6;
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  outline: none;
  resize: vertical;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.field-textarea:focus {
  border-color: var(--primary);
}

/* ---------- 头像选择 ---------- */
.avatar-picker {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.avatar-opt {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  font-size: 18px;
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard);
}

.avatar-opt:hover {
  border-color: var(--primary);
}

.avatar-opt.active {
  background: rgba(74, 126, 255, 0.14);
  border-color: var(--primary);
  transform: scale(1.05);
}

/* ---------- 工具开关行 ---------- */
.switch-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.switch-label {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.dialog-delete-content {
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  padding: 4px 0;
}
</style>