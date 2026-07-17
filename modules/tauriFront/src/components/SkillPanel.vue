<script setup lang="ts">
/**
 * SkillPanel 技能管理面板
 * 列出内置 + 用户技能，支持创建 / 编辑 / 删除 / 应用到当前会话。
 * 容器复用 BindSheet side="right"。
 */
import { ref, computed, watch, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  BindSheet,
  Button,
  Chips,
  Menu,
  Dialog,
  useToast,
  type MenuItemOption,
} from './basic'
import type { Skill } from '../types'

const props = defineProps<{ open: boolean; conversationId: string | null }>()
const emit = defineEmits<{ (e: 'close'): void }>()

const { toast } = useToast()

// ---------- 数据 ----------
const skills = ref<Skill[]>([])
const loading = ref(false)

// 内置技能的视觉映射（覆盖后端返回的描述以突出能力说明）
const builtinVisuals: Record<string, { glyph: string; desc: string; accent: string }> = {
  'agent-reach': {
    glyph: '🌐',
    desc: '互联网访问能力，可搜索 Twitter/YouTube/B站/Reddit 等',
    accent: '#4a7eff',
  },
  'browser-act': {
    glyph: '🖥',
    desc: '浏览器自动化，抓取页面、表单填写、截图',
    accent: '#10a37f',
  },
}

function glyphOf(s: Skill): string {
  if (s.builtin && builtinVisuals[s.id]) return builtinVisuals[s.id].glyph
  return '✦'
}

function descOf(s: Skill): string {
  if (s.builtin && builtinVisuals[s.id]) return builtinVisuals[s.id].desc
  return s.description || '用户自定义技能'
}

function accentOf(s: Skill): string {
  if (s.builtin && builtinVisuals[s.id]) return builtinVisuals[s.id].accent
  return '#7a8190'
}

// ---------- 工具选项 ----------
const toolOptions: { key: string; label: string; icon: string }[] = [
  { key: 'search_history', label: '搜索历史', icon: '🔍' },
  { key: 'get_time', label: '获取时间', icon: '🕐' },
  { key: 'read_file', label: '读取文件', icon: '📄' },
  { key: 'list_files', label: '列出文件', icon: '📁' },
  { key: 'shell', label: '执行命令', icon: '⌨' },
  { key: 'web_fetch', label: '网页抓取', icon: '🌐' },
]

const toolLabelMap = new Map(toolOptions.map((t) => [t.key, t.label]))

function toolLabel(key: string): string {
  return toolLabelMap.get(key) ?? key
}

// ---------- 加载 ----------
async function refresh() {
  loading.value = true
  try {
    skills.value = await invoke<Skill[]>('list_skills')
  } catch (e) {
    toast({ content: `加载技能失败：${e}`, type: 'error' })
    skills.value = []
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

const userSkills = computed(() => skills.value.filter((s) => !s.builtin))
const builtinSkills = computed(() => skills.value.filter((s) => s.builtin))

// ---------- 应用技能 ----------
async function applySkill(s: Skill) {
  if (!props.conversationId) {
    toast({ content: '请先选择或新建一个会话再应用技能', type: 'warn' })
    return
  }
  try {
    await invoke('apply_skill', {
      conversationId: props.conversationId,
      skillId: s.id,
    })
    toast({ content: `已应用技能「${s.name}」到当前会话`, type: 'success' })
  } catch (e) {
    toast({ content: `应用失败：${e}`, type: 'error' })
  }
}

// ---------- 卡片菜单（编辑/删除） ----------
const menuOpen = ref(false)
const menuTriggerEl = ref<HTMLElement | null>(null)
const menuSkill = ref<Skill | null>(null)

function openSkillMenu(s: Skill, e: MouseEvent) {
  menuSkill.value = s
  menuTriggerEl.value = e.currentTarget as HTMLElement
  menuOpen.value = true
}

const menuItems = computed<MenuItemOption[]>(() => [
  { key: 'edit', label: '编辑', icon: '✎' },
  { key: 'delete', label: '删除', icon: '🗑', danger: true, divided: true },
])

function onMenuSelect(item: MenuItemOption) {
  const s = menuSkill.value
  menuSkill.value = null
  if (!s) return
  if (item.key === 'edit') startEdit(s)
  else if (item.key === 'delete') {
    deleteTarget.value = s
    deleteDialogOpen.value = true
  }
}

// ---------- 新建 / 编辑 Dialog ----------
const dialogOpen = ref(false)
const editingId = ref<string | null>(null)

const draft = ref({
  name: '',
  description: '',
  preamble: '',
  tools: [] as string[],
})

function newId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID()
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

function openCreate() {
  editingId.value = null
  draft.value = { name: '', description: '', preamble: '', tools: [] }
  dialogOpen.value = true
}

function startEdit(s: Skill) {
  editingId.value = s.id
  draft.value = {
    name: s.name,
    description: s.description,
    preamble: s.preamble,
    tools: [...s.tools],
  }
  dialogOpen.value = true
}

function toggleTool(key: string, selected: boolean) {
  if (selected) {
    if (!draft.value.tools.includes(key)) draft.value.tools.push(key)
  } else {
    draft.value.tools = draft.value.tools.filter((t) => t !== key)
  }
}

const dialogTitle = computed(() => (editingId.value ? '编辑技能' : '新建技能'))

async function confirmSave() {
  const name = draft.value.name.trim()
  if (!name) {
    toast({ content: '请输入技能名称', type: 'warn' })
    return
  }
  try {
    const skill: Skill = {
      id: editingId.value ?? newId(),
      name,
      description: draft.value.description.trim(),
      preamble: draft.value.preamble,
      tools: [...draft.value.tools],
      created_at: 0,
      builtin: false,
    }
    if (editingId.value) {
      await invoke('update_skill', { id: editingId.value, skill })
      toast({ content: `已更新技能「${name}」`, type: 'success' })
    } else {
      await invoke('create_skill', { skill })
      toast({ content: `已创建技能「${name}」`, type: 'success' })
    }
    dialogOpen.value = false
    await refresh()
  } catch (e) {
    toast({ content: `保存失败：${e}`, type: 'error' })
  }
}

// ---------- 删除 ----------
const deleteDialogOpen = ref(false)
const deleteTarget = ref<Skill | null>(null)

async function confirmDelete() {
  const s = deleteTarget.value
  if (!s) return
  try {
    await invoke('delete_skill', { id: s.id })
    toast({ content: `已删除技能「${s.name}」`, type: 'success' })
    await refresh()
  } catch (e) {
    toast({ content: `删除失败：${e}`, type: 'error' })
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
    title="技能管理"
    @close="onClose"
  >
    <div class="skill-body">
      <!-- 顶部说明 -->
      <header class="skill-hero">
        <div class="hero-mark">⚡</div>
        <div class="hero-text">
          <h2 class="hero-title">技能</h2>
          <p class="hero-sub">点击技能卡片可应用到当前会话；内置技能提供联网与浏览器能力</p>
        </div>
      </header>

      <!-- 内置技能 -->
      <section v-if="builtinSkills.length" class="section">
        <div class="section-head">
          <span class="section-title">内置技能</span>
          <span class="count-badge">{{ builtinSkills.length }}</span>
        </div>
        <div class="skill-list">
          <div
            v-for="s in builtinSkills"
            :key="s.id"
            class="skill-card builtin"
            @click="applySkill(s)"
          >
            <span class="skill-glyph" :style="{ background: accentOf(s) }">{{ glyphOf(s) }}</span>
            <div class="skill-info">
              <div class="skill-top">
                <span class="skill-name">{{ s.name }}</span>
                <span class="builtin-badge">内置</span>
              </div>
              <p class="skill-desc">{{ descOf(s) }}</p>
              <div v-if="s.tools.length" class="skill-tools">
                <Chips
                  v-for="t in s.tools"
                  :key="t"
                  :label="toolLabel(t)"
                  size="sm"
                />
              </div>
            </div>
          </div>
        </div>
      </section>

      <!-- 用户技能 -->
      <section class="section">
        <div class="section-head">
          <span class="section-title">我的技能</span>
          <span v-if="userSkills.length" class="count-badge">{{ userSkills.length }}</span>
        </div>

        <div v-if="!userSkills.length && !loading" class="empty-state">
          <div class="empty-illust">✦</div>
          <p class="empty-text">还没有自定义技能</p>
          <p class="empty-hint">点击下方按钮，创建你的第一个技能</p>
        </div>

        <div v-else class="skill-list">
          <div
            v-for="s in userSkills"
            :key="s.id"
            class="skill-card"
            @click="applySkill(s)"
          >
            <div class="skill-card-main">
              <span class="skill-glyph" :style="{ background: accentOf(s) }">{{ glyphOf(s) }}</span>
              <div class="skill-info">
                <div class="skill-top">
                  <span class="skill-name">{{ s.name }}</span>
                </div>
                <p class="skill-desc">{{ descOf(s) }}</p>
                <div v-if="s.tools.length" class="skill-tools">
                  <Chips
                    v-for="t in s.tools"
                    :key="t"
                    :label="toolLabel(t)"
                    size="sm"
                  />
                </div>
              </div>
            </div>
            <button
              type="button"
              class="skill-card-menu"
              title="更多操作"
              @click.stop="(e) => openSkillMenu(s, e)"
            >⋯</button>
          </div>
        </div>
      </section>

      <!-- 底部新建按钮 -->
      <div class="skill-footer">
        <Button variant="primary" block @click="openCreate">
          <template #icon>＋</template>
          新建技能
        </Button>
      </div>
    </div>

    <!-- 卡片操作菜单 -->
    <Menu
      :visible="menuOpen"
      :items="menuItems"
      :trigger-ref="menuTriggerEl"
      placement="bottom-end"
      @update:visible="menuOpen = $event"
      @select="onMenuSelect"
    />

    <!-- 新建 / 编辑 Dialog -->
    <Dialog
      v-model:visible="dialogOpen"
      :title="dialogTitle"
      :confirm-text="editingId ? '保存' : '创建'"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmSave"
    >
      <div class="form-body">
        <div class="field">
          <label class="field-label">名称</label>
          <input
            v-model="draft.name"
            type="text"
            class="field-input"
            placeholder="例如：翻译助手"
          />
        </div>
        <div class="field">
          <label class="field-label">描述</label>
          <input
            v-model="draft.description"
            type="text"
            class="field-input"
            placeholder="一句话说明技能用途"
          />
        </div>
        <div class="field">
          <label class="field-label">系统提示词前缀（preamble）</label>
          <textarea
            v-model="draft.preamble"
            rows="4"
            class="field-input field-textarea"
            placeholder="作为系统消息注入会话，定义 agent 行为约束"
          ></textarea>
        </div>
        <div class="field">
          <label class="field-label">启用工具</label>
          <div class="tools-grid">
            <Chips
              v-for="opt in toolOptions"
              :key="opt.key"
              :label="opt.label"
              :icon="opt.icon"
              :selected="draft.tools.includes(opt.key)"
              size="sm"
              @update:selected="toggleTool(opt.key, $event)"
            />
          </div>
        </div>
      </div>
    </Dialog>

    <!-- 删除确认 -->
    <Dialog
      v-model:visible="deleteDialogOpen"
      title="删除技能"
      danger
      confirm-text="删除"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmDelete"
    >
      <div class="dialog-delete-content">
        确定删除技能「{{ deleteTarget?.name }}」？此操作不可撤销。
      </div>
    </Dialog>
  </BindSheet>
</template>

<style scoped>
.skill-body {
  padding: 20px 24px 32px;
  display: flex;
  flex-direction: column;
  gap: 20px;
  overflow-y: auto;
}

/* ---------- 顶部说明 ---------- */
.skill-hero {
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

/* ---------- 技能卡片 ---------- */
.skill-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.skill-card {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--card);
  cursor: pointer;
  transition: border-color var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard),
    box-shadow var(--duration-fast) var(--ease-standard);
}

.skill-card:hover {
  border-color: var(--primary);
  background: var(--card-2);
}

.skill-card.builtin {
  background: linear-gradient(135deg, rgba(74, 126, 255, 0.06), transparent);
}

.skill-card.builtin:hover {
  box-shadow: 0 0 0 1px var(--primary);
}

.skill-card-main {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  flex: 1;
  min-width: 0;
}

.skill-glyph {
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

.skill-info {
  flex: 1;
  min-width: 0;
}

.skill-top {
  display: flex;
  align-items: center;
  gap: 8px;
}

.skill-name {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.builtin-badge {
  flex-shrink: 0;
  padding: 1px 8px;
  font-size: var(--fs-xs);
  color: #fff;
  background: var(--primary);
  border-radius: var(--radius-full);
}

.skill-desc {
  margin: 4px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.5;
}

.skill-tools {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}

.skill-card-menu {
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

.skill-card-menu:hover {
  background: var(--card-2);
  color: var(--text);
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

/* ---------- 底部 ---------- */
.skill-footer {
  padding-top: 4px;
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
  min-height: 92px;
  padding: 10px 12px;
  resize: vertical;
  line-height: 1.5;
  font-family: inherit;
}

.tools-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.dialog-delete-content {
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  padding: 4px 0;
}
</style>
