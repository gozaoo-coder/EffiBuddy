<script setup lang="ts">
/**
 * AgentTeamView 智能体群聊视图（WeChat 风格）
 * - 左侧：群列表（群名 + 成员数 + 最近消息时间）
 * - 右侧：聊天窗口（消息流 + @ 提及 + 任务颁布 + 成员管理）
 * - 监听后端 "agent-team-event" 实时刷新当前群
 * 自包含组件（无 props / emits），数据全部来自 Tauri 命令。
 */
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  Icon,
  Button,
  IconButton,
  Dialog,
  Chips,
  useToast,
} from '../basic'
import type { AgentTeam, AgentDef, TeamMember, TeamMessage } from '../../types'

const { toast } = useToast()

/** 当前登录用户（后端约定）在群成员 / 消息中的 id */
const MY_ID = 'user:me'
/** 主智能体作为成员时的 id（add_team_member 约定） */
const MAIN_MEMBER_ID = 'main'

// =========================================================
// 数据
// =========================================================
const teams = ref<AgentTeam[]>([])
const agentDefs = ref<AgentDef[]>([])
const loading = ref(false)
const activeTeamId = ref('')

const activeTeam = computed<AgentTeam | null>(
  () => teams.value.find((t) => t.id === activeTeamId.value) ?? null,
)

/** 当前用户是否为管理员（owner 或 admin） */
const isAdmin = computed(() => {
  const team = activeTeam.value
  if (!team) return false
  if (team.owner_id === MY_ID) return true
  const me = team.members.find((m) => m.id === MY_ID)
  return me?.role === 'owner' || me?.role === 'admin'
})

async function refresh() {
  loading.value = true
  try {
    const [t, d] = await Promise.all([
      invoke<AgentTeam[]>('list_agent_teams'),
      invoke<AgentDef[]>('list_agent_defs'),
    ])
    teams.value = t
    agentDefs.value = d
    // 若当前选中的群被删除，回退到第一个群
    if (!teams.value.some((x) => x.id === activeTeamId.value)) {
      activeTeamId.value = teams.value[0]?.id ?? ''
    }
  } catch (e) {
    toast({ content: `加载群聊失败：${e}`, type: 'error' })
  } finally {
    loading.value = false
  }
}

function selectTeam(id: string) {
  if (activeTeamId.value === id) return
  activeTeamId.value = id
  mentions.value = []
}

// =========================================================
// 时间格式化
// =========================================================
function formatTime(ts: number | null | undefined): string {
  if (!ts) return ''
  const diff = Date.now() - ts
  if (diff < 60000) return '刚刚'
  if (diff < 3600000) return `${Math.floor(diff / 60000)} 分钟前`
  if (diff < 86400000) return `${Math.floor(diff / 3600000)} 小时前`
  if (diff < 2592000000) return `${Math.floor(diff / 86400000)} 天前`
  try {
    return new Date(ts).toLocaleString()
  } catch {
    return ''
  }
}

function formatClock(ts: number): string {
  try {
    return new Date(ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  } catch {
    return ''
  }
}

// =========================================================
// 新建群
// =========================================================
const createOpen = ref(false)
const createDraft = ref({
  name: '',
  description: '',
  includeMain: true,
  selectedDefIds: [] as string[],
})

function openCreate() {
  createDraft.value = { name: '', description: '', includeMain: true, selectedDefIds: [] }
  createOpen.value = true
}

function toggleCreateDef(id: string) {
  const list = createDraft.value.selectedDefIds
  const i = list.indexOf(id)
  if (i >= 0) list.splice(i, 1)
  else list.push(id)
}

async function confirmCreate() {
  const name = createDraft.value.name.trim()
  if (!name) {
    toast({ content: '请填写群名称', type: 'warn' })
    return
  }
  try {
    const team: AgentTeam = {
      id: '',
      name,
      description: createDraft.value.description.trim(),
      owner_id: '',
      members: [],
      messages: [],
      created_at: 0,
      updated_at: 0,
    }
    const created = await invoke<AgentTeam>('save_agent_team', { team })
    const teamId = created.id
    // 加入主 agent
    if (createDraft.value.includeMain) {
      await invoke('add_team_member', {
        teamId,
        memberId: MAIN_MEMBER_ID,
        name: '主智能体',
        avatar: '🤖',
        role: 'member',
      })
    }
    // 加入勾选的自定义智能体
    for (const defId of createDraft.value.selectedDefIds) {
      const def = agentDefs.value.find((d) => d.id === defId)
      if (!def) continue
      await invoke('add_team_member', {
        teamId,
        memberId: `def:${def.id}`,
        name: def.name,
        avatar: def.avatar || '🤖',
        role: 'member',
      })
    }
    toast({ content: `已创建群「${name}」`, type: 'success' })
    createOpen.value = false
    await refresh()
    activeTeamId.value = teamId
  } catch (e) {
    toast({ content: `创建群失败：${e}`, type: 'error' })
  }
}

// =========================================================
// 聊天消息
// =========================================================
const draftMessage = ref('')
const mentions = ref<string[]>([])
const msgListEl = ref<HTMLElement | null>(null)

const isOwnMessage = (m: TeamMessage) => m.sender_id === MY_ID

function scrollToBottom() {
  nextTick(() => {
    if (msgListEl.value) msgListEl.value.scrollTop = msgListEl.value.scrollHeight
  })
}

// 消息数量变化后滚动到底部（新消息进入）
watch(
  () => activeTeam.value?.messages.length,
  () => scrollToBottom(),
)

async function sendMessage() {
  const content = draftMessage.value.trim()
  const team = activeTeam.value
  if (!team || !content) return
  const m = [...mentions.value]
  try {
    await invoke('send_team_message', {
      teamId: team.id,
      content,
      mentions: m,
      kind: 'text',
    })
    draftMessage.value = ''
    mentions.value = []
    await refresh()
  } catch (e) {
    toast({ content: `发送失败：${e}`, type: 'error' })
  }
}

// =========================================================
// @ 提及（float 成员列表）
// =========================================================
const mentionOpen = ref(false)
const mentionListEl = ref<HTMLElement | null>(null)
const inputEl = ref<HTMLTextAreaElement | null>(null)

function toggleMention() {
  if (!activeTeam.value) return
  mentionOpen.value = !mentionOpen.value
}

function pickMention(member: TeamMember) {
  const tag = `@${member.name} `
  const el = inputEl.value
  if (el) {
    const start = el.selectionStart ?? draftMessage.value.length
    const end = el.selectionEnd ?? start
    draftMessage.value = draftMessage.value.slice(0, start) + tag + draftMessage.value.slice(end)
    nextTick(() => {
      el.focus()
      const pos = start + tag.length
      el.setSelectionRange(pos, pos)
    })
  } else {
    draftMessage.value += tag
  }
  if (!mentions.value.includes(member.id)) mentions.value.push(member.id)
  mentionOpen.value = false
}

function onKeydownInput(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault()
    sendMessage()
  }
}

// 点击外部关闭 @ 列表
function onDocClick(e: MouseEvent) {
  const t = e.target as Node
  if (mentionListEl.value?.contains(t)) return
  if (mentionOpen.value) mentionOpen.value = false
}

// =========================================================
// 成员管理
// =========================================================
const memberOpen = ref(false)
const addMemberOpen = ref(false)

function openMembers() {
  memberOpen.value = true
}

function closeMembers() {
  memberOpen.value = false
}

/** 尚未加入当前群的自定义智能体（用于"添加成员"） */
const addableDefs = computed<AgentDef[]>(() => {
  const team = activeTeam.value
  if (!team) return []
  return agentDefs.value.filter((d) => !team.members.some((m) => m.agent_def_id === d.id))
})

function openAddMember() {
  addMemberOpen.value = true
}

async function addDefMember(def: AgentDef) {
  const team = activeTeam.value
  if (!team) return
  try {
    await invoke('add_team_member', {
      teamId: team.id,
      memberId: `def:${def.id}`,
      name: def.name,
      avatar: def.avatar || '🤖',
      role: 'member',
    })
    toast({ content: `已加入「${def.name}」`, type: 'success' })
    await refresh()
  } catch (e) {
    toast({ content: `添加成员失败：${e}`, type: 'error' })
  }
}

async function removeMember(member: TeamMember) {
  const team = activeTeam.value
  if (!team) return
  try {
    await invoke('remove_team_member', { teamId: team.id, memberId: member.id })
    toast({ content: `已移除「${member.name}」`, type: 'success' })
    await refresh()
  } catch (e) {
    toast({ content: `移除成员失败：${e}`, type: 'error' })
  }
}

/** 成员是否可被移除：非 owner、非我自己 */
const canRemoveMember = (m: TeamMember) => m.id !== activeTeam.value?.owner_id && m.id !== MY_ID

const roleLabel: Record<string, string> = {
  owner: '群主',
  admin: '管理员',
  member: '成员',
}

// =========================================================
// 颁布任务
// =========================================================
const taskOpen = ref(false)
const taskDraft = ref({ content: '', assigneeIds: [] as string[] })

/** 可指派任务的 agent 成员（agent / main_agent） */
const taskableMembers = computed<TeamMember[]>(() =>
  (activeTeam.value?.members ?? []).filter((m) => m.kind === 'agent' || m.kind === 'main_agent'),
)

function openTask() {
  const all = taskableMembers.value.map((m) => m.id)
  taskDraft.value = { content: '', assigneeIds: all }
  taskOpen.value = true
}

function toggleAssignee(id: string) {
  const list = taskDraft.value.assigneeIds
  const i = list.indexOf(id)
  if (i >= 0) list.splice(i, 1)
  else list.push(id)
}

async function confirmTask() {
  const content = taskDraft.value.content.trim()
  const team = activeTeam.value
  if (!team || !content) {
    toast({ content: '请填写任务内容', type: 'warn' })
    return
  }
  try {
    await invoke('assign_team_task', {
      teamId: team.id,
      content,
      assignees: taskDraft.value.assigneeIds,
    })
    toast({ content: '任务已颁布', type: 'success' })
    taskOpen.value = false
    await refresh()
  } catch (e) {
    toast({ content: `颁布任务失败：${e}`, type: 'error' })
  }
}

// =========================================================
// 删除群
// =========================================================
const deleteOpen = ref(false)

function askDelete() {
  deleteOpen.value = true
}

async function confirmDelete() {
  const team = activeTeam.value
  if (!team) return
  try {
    await invoke('delete_agent_team', { id: team.id })
    toast({ content: `已删除群「${team.name}」`, type: 'success' })
    deleteOpen.value = false
    await refresh()
  } catch (e) {
    toast({ content: `删除失败：${e}`, type: 'error' })
  }
}

// =========================================================
// 监听后端事件
// =========================================================
let unlistens: UnlistenFn[] = []

async function setupListeners() {
  unlistens.push(
    await listen<{ type: string; team: AgentTeam }>('agent-team-event', (e) => {
      const { type, team } = e.payload
      // 仅刷新当前正在查看的群
      if (team.id === activeTeamId.value) {
        refresh()
      } else {
        // 其他群：仅刷新列表以更新最近消息摘要
        refresh()
      }
      if (type === 'message' || type === 'task') {
        toast({ content: `「${team.name}」有新消息`, type: 'info' })
      }
    }),
  )
}

onMounted(() => {
  refresh()
  setupListeners()
  if (typeof document !== 'undefined') {
    document.addEventListener('click', onDocClick)
  }
})

onUnmounted(() => {
  unlistens.forEach((fn) => fn?.())
  unlistens = []
  if (typeof document !== 'undefined') {
    document.removeEventListener('click', onDocClick)
  }
})
</script>

<template>
  <div class="agent-team-view">
    <!-- ============ 左侧：群列表 ============ -->
    <aside class="team-rail">
      <header class="rail-head">
        <span class="rail-title">智能体群聊</span>
        <IconButton size="sm" title="新建群" @click="openCreate">
          <Icon name="plus" :size="18" />
        </IconButton>
      </header>

      <div v-if="!teams.length && !loading" class="rail-empty">
        <div class="rail-empty-icon"><Icon name="chat" :size="40" /></div>
        <p class="rail-empty-text">还没有群</p>
        <Button size="sm" variant="primary" @click="openCreate">新建群</Button>
      </div>

      <div v-else class="rail-list">
        <button
          v-for="t in teams"
          :key="t.id"
          type="button"
          class="team-item"
          :class="{ 'is-active': t.id === activeTeamId }"
          @click="selectTeam(t.id)"
        >
          <div class="team-item-main">
            <span class="team-name">{{ t.name }}</span>
            <span class="team-meta">{{ t.members.length }} 成员 · {{ formatTime(t.updated_at) }}</span>
          </div>
        </button>
      </div>
    </aside>

    <!-- ============ 右侧：聊天窗口 ============ -->
    <section class="chat-area">
      <template v-if="activeTeam">
        <!-- 顶部：群名 + 成员头像 -->
        <header class="chat-head">
          <div class="head-title-block">
            <span class="head-title">{{ activeTeam.name }}</span>
            <span v-if="activeTeam.description" class="head-desc">{{ activeTeam.description }}</span>
          </div>
          <button type="button" class="head-avatars" title="成员管理" @click="openMembers">
            <span
              v-for="m in activeTeam.members"
              :key="m.id"
              class="avatar-chip"
              :title="m.name"
            >{{ m.avatar || '🤖' }}</span>
            <span class="avatar-count">+{{ activeTeam.members.length }}</span>
          </button>
        </header>

        <!-- 消息流 -->
        <div ref="msgListEl" class="chat-messages">
          <TransitionGroup name="msg" tag="div" class="msg-list">
            <div
              v-for="m in activeTeam.messages"
              :key="m.id"
              class="msg-row"
              :class="{ 'is-mine': isOwnMessage(m), 'is-system': m.kind === 'system' }"
            >
              <!-- system 消息：居中灰字 -->
              <div v-if="m.kind === 'system'" class="msg-system">{{ m.content }}</div>

              <!-- 任务卡片 -->
              <div v-else-if="m.kind === 'task'" class="msg-task">
                <div class="task-badge"><Icon name="bolt" :size="14" /> 任务</div>
                <div class="task-content">{{ m.content }}</div>
                <div v-if="m.mentions && m.mentions.length" class="task-assignees">
                  <span v-for="id in m.mentions" :key="id" class="task-at">@{{ id }}</span>
                </div>
                <div class="task-status">
                  <span v-if="m.reply" class="task-reply">回复：{{ m.reply }}</span>
                  <span v-else class="task-state" :class="{ done: m.task_handled }">
                    {{ m.task_handled ? '已处理' : '待处理' }}
                  </span>
                </div>
              </div>

              <!-- 普通消息（text / 非 mine 或 mine） -->
              <template v-else>
                <div class="msg-avatar">{{ m.sender_avatar || '🧑' }}</div>
                <div class="msg-body">
                  <div class="msg-meta">
                    <span class="msg-name">{{ m.sender_name }}</span>
                    <span class="msg-time">{{ formatClock(m.created_at) }}</span>
                  </div>
                  <div class="msg-bubble">{{ m.content }}</div>
                </div>
              </template>
            </div>
          </TransitionGroup>
        </div>

        <!-- 输入区 -->
        <div class="chat-input">
          <div class="input-bar">
            <button type="button" class="at-btn" title="提及成员" @click="toggleMention">
              <Icon name="chat" :size="18" />
            </button>
            <textarea
              ref="inputEl"
              v-model="draftMessage"
              class="input-box"
              rows="2"
              placeholder="输入消息，Enter 发送，Shift+Enter 换行"
              @keydown="onKeydownInput"
            ></textarea>
            <Button variant="primary" size="md" :disabled="!draftMessage.trim()" @click="sendMessage">
              <template #icon><Icon name="send" :size="18" /></template>
              发送
            </Button>
          </div>
          <div v-if="mentions.length" class="mention-tags">
            <Chips
              v-for="id in mentions"
              :key="id"
              :label="'@' + (activeTeam.members.find((me) => me.id === id)?.name ?? id)"
              size="sm"
              removable
              @remove="mentions = mentions.filter((x) => x !== id)"
            />
          </div>

          <!-- @ 成员 float 列表 -->
          <Transition name="pop">
            <div v-if="mentionOpen" ref="mentionListEl" class="mention-float" @click.stop>
              <button
                v-for="m in activeTeam.members"
                :key="m.id"
                type="button"
                class="mention-item"
                @click="pickMention(m)"
              >
                <span class="mention-avatar">{{ m.avatar || '🤖' }}</span>
                <span class="mention-name">{{ m.name }}</span>
              </button>
            </div>
          </Transition>
        </div>
      </template>

      <!-- 未选中群 -->
      <div v-else class="chat-empty">
        <div class="chat-empty-icon"><Icon name="chat" :size="48" /></div>
        <p class="chat-empty-text">选择一个群开始聊天</p>
        <p v-if="!teams.length" class="chat-empty-hint">点击左侧「新建群」创建你的第一个智能体群</p>
      </div>
    </section>

    <!-- ============ 新建群 Dialog ============ -->
    <Dialog
      v-model:visible="createOpen"
      title="新建智能体群"
      confirm-text="创建"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmCreate"
    >
      <div class="form-body">
        <div class="field">
          <label class="field-label">群名称</label>
          <input v-model="createDraft.name" type="text" class="field-input" placeholder="例如：产品研发小分队" />
        </div>
        <div class="field">
          <label class="field-label">群描述（可选）</label>
          <input v-model="createDraft.description" type="text" class="field-input" placeholder="一句话说明群的用途" />
        </div>
        <div class="field">
          <label class="field-label">成员</label>
          <div class="member-checks">
            <Chips
              label="主智能体"
              icon="🤖"
              :selected="createDraft.includeMain"
              size="md"
              @click="createDraft.includeMain = !createDraft.includeMain"
            />
            <Chips
              v-for="d in agentDefs"
              :key="d.id"
              :label="d.name"
              :icon="d.avatar || '🤖'"
              :selected="createDraft.selectedDefIds.includes(d.id)"
              size="md"
              @click="toggleCreateDef(d.id)"
            />
          </div>
        </div>
      </div>
    </Dialog>

    <!-- ============ 成员管理 Dialog ============ -->
    <Dialog
      v-model:visible="memberOpen"
      title="群成员"
      :show-confirm="false"
      cancel-text="关闭"
      @cancel="closeMembers"
    >
      <div class="member-list">
        <div v-for="m in activeTeam?.members" :key="m.id" class="member-row">
          <span class="member-avatar">{{ m.avatar || '🤖' }}</span>
          <div class="member-info">
            <span class="member-name">
              {{ m.name }}
              <span v-if="m.id === MY_ID" class="me-badge">我</span>
            </span>
            <span class="member-role">{{ roleLabel[m.role] ?? m.role }}</span>
          </div>
          <IconButton
            v-if="canRemoveMember(m)"
            size="sm"
            variant="danger"
            title="移除成员"
            @click="removeMember(m)"
          ><Icon name="minus" :size="16" /></IconButton>
        </div>
        <div v-if="!activeTeam?.members.length" class="member-empty">暂无成员</div>
      </div>
      <div class="member-footer">
        <Button variant="normal" size="sm" block @click="openAddMember">
          <template #icon><Icon name="plus" :size="16" /></template>
          添加成员
        </Button>
      </div>
    </Dialog>

    <!-- ============ 添加成员 Dialog ============ -->
    <Dialog
      v-model:visible="addMemberOpen"
      title="添加智能体"
      :show-confirm="false"
      cancel-text="关闭"
      width="460px"
    >
      <div v-if="!addableDefs.length" class="member-empty">没有可添加的智能体</div>
      <div v-else class="addable-list">
        <div v-for="d in addableDefs" :key="d.id" class="addable-row">
          <span class="member-avatar">{{ d.avatar || '🤖' }}</span>
          <div class="member-info">
            <span class="member-name">{{ d.name }}</span>
            <span class="member-role">{{ d.role || '智能体' }}</span>
          </div>
          <Button size="sm" variant="primary" @click="addDefMember(d)">加入</Button>
        </div>
      </div>
    </Dialog>

    <!-- ============ 颁布任务 Dialog（管理员） ============ -->
    <Dialog
      v-if="isAdmin"
      v-model:visible="taskOpen"
      title="颁布任务"
      confirm-text="颁布"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmTask"
    >
      <div class="form-body">
        <div class="field">
          <label class="field-label">任务内容</label>
          <textarea v-model="taskDraft.content" class="field-input taskarea" rows="3" placeholder="描述要交给智能体的任务"></textarea>
        </div>
        <div class="field">
          <label class="field-label">指派给（不选则 @全体）</label>
          <div class="member-checks">
            <Chips
              v-for="m in taskableMembers"
              :key="m.id"
              :label="m.name"
              :icon="m.avatar || '🤖'"
              :selected="taskDraft.assigneeIds.includes(m.id)"
              size="md"
              @click="toggleAssignee(m.id)"
            />
          </div>
        </div>
      </div>
    </Dialog>

    <!-- ============ 删除群确认 Dialog（管理员） ============ -->
    <Dialog
      v-if="isAdmin"
      v-model:visible="deleteOpen"
      title="删除群"
      danger
      confirm-text="删除"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmDelete"
    >
      <div class="dialog-delete-content">
        确定删除群「{{ activeTeam?.name }}」？此操作不可撤销。
      </div>
    </Dialog>
  </div>

  <!-- 管理员操作栏（悬浮于聊天区右上） -->
  <div v-if="isAdmin && activeTeam" class="admin-actions">
    <Button size="sm" variant="normal" @click="openTask">
      <template #icon><Icon name="bolt" :size="16" /></template>
      颁布任务
    </Button>
    <IconButton size="sm" variant="danger" title="删除群" @click="askDelete">
      <Icon name="delete" :size="16" />
    </IconButton>
  </div>
</template>

<style scoped>
.agent-team-view {
  display: flex;
  height: 100%;
  overflow: hidden;
  position: relative;
}

/* ============ 群列表 ============ */
.team-rail {
  width: 200px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border);
  background: var(--bg-2);
}

.rail-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 12px 8px;
}

.rail-title {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.rail-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 20px;
  color: var(--muted);
}

.rail-empty-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: 50%;
  background: var(--card-2);
  color: var(--muted);
}

.rail-empty-text {
  margin: 0;
  font-size: var(--fs-base);
  color: var(--text);
}

.rail-list {
  flex: 1;
  overflow-y: auto;
  padding: 4px 6px 12px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.team-item {
  display: flex;
  align-items: center;
  width: 100%;
  min-height: 44px;
  padding: 8px 10px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
}

.team-item:hover {
  background: var(--card);
}

.team-item.is-active {
  background: var(--card);
  box-shadow: inset 2px 0 0 var(--primary);
}

.team-item-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.team-name {
  font-size: var(--fs-base);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.team-meta {
  font-size: var(--fs-xs);
  color: var(--muted);
}

/* ============ 聊天区 ============ */
.chat-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  background: var(--bg);
}

.chat-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 16px;
  border-bottom: 1px solid var(--border);
  background: var(--card);
}

.head-title-block {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.head-title {
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text);
}

.head-desc {
  font-size: var(--fs-xs);
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.head-avatars {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  background: var(--card-2);
  cursor: pointer;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.head-avatars:hover {
  border-color: var(--primary);
}

.avatar-chip {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: 50%;
  background: var(--bg);
  font-size: 14px;
}

.avatar-count {
  font-size: var(--fs-xs);
  color: var(--muted);
}

/* ============ 消息流 ============ */
.chat-messages {
  flex: 1;
  overflow-y: auto;
  padding: 16px;
}

.msg-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.msg-row {
  display: flex;
  gap: 10px;
}

.msg-row.is-mine {
  flex-direction: row-reverse;
}

.msg-avatar {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: 50%;
  background: var(--card-2);
  font-size: 18px;
}

.msg-body {
  max-width: 68%;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.msg-row.is-mine .msg-body {
  align-items: flex-end;
}

.msg-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 4px;
}

.msg-row.is-mine .msg-meta {
  flex-direction: row-reverse;
}

.msg-name {
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--muted);
}

.msg-time {
  font-size: var(--fs-xs);
  color: var(--muted);
}

.msg-bubble {
  padding: 8px 12px;
  border-radius: var(--radius-lg);
  background: var(--card-2);
  color: var(--text);
  font-size: var(--fs-base);
  line-height: 1.5;
  word-break: break-word;
  white-space: pre-wrap;
}

.msg-row.is-mine .msg-bubble {
  background: var(--primary);
  color: #fff;
  border-top-right-radius: 2px;
}

.msg-row:not(.is-mine) .msg-bubble {
  border-top-left-radius: 2px;
}

/* system 消息 */
.msg-system {
  align-self: center;
  padding: 4px 12px;
  font-size: var(--fs-xs);
  color: var(--muted);
  background: var(--card);
  border-radius: var(--radius-full);
}

/* 任务卡片 */
.msg-task {
  max-width: 68%;
  align-self: flex-start;
  padding: 10px 12px;
  border: 1px solid var(--primary);
  border-radius: var(--radius-lg);
  background: rgba(74, 126, 255, 0.08);
}

.task-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--primary);
  margin-bottom: 6px;
}

.task-content {
  font-size: var(--fs-base);
  color: var(--text);
  line-height: 1.5;
  word-break: break-word;
  white-space: pre-wrap;
}

.task-assignees {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}

.task-at {
  padding: 1px 8px;
  font-size: var(--fs-xs);
  color: var(--primary);
  background: rgba(74, 126, 255, 0.12);
  border-radius: var(--radius-full);
}

.task-status {
  margin-top: 8px;
  font-size: var(--fs-xs);
  color: var(--muted);
}

.task-reply {
  color: var(--text);
}

.task-state {
  padding: 1px 8px;
  border-radius: var(--radius-full);
  background: var(--card-2);
}

.task-state.done {
  color: var(--primary);
}

/* 消息进入动画 */
.msg-enter-active {
  transition: opacity var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard);
}

.msg-enter-from {
  opacity: 0;
  transform: translateY(8px);
}

/* ============ 输入区 ============ */
.chat-input {
  position: relative;
  padding: 10px 16px 14px;
  border-top: 1px solid var(--border);
  background: var(--card);
}

.input-bar {
  display: flex;
  align-items: flex-end;
  gap: 10px;
}

.at-btn {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--card-2);
  color: var(--muted);
  font-size: 18px;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.at-btn:hover {
  color: var(--primary);
  border-color: var(--primary);
}

.input-box {
  flex: 1;
  resize: none;
  padding: 8px 12px;
  font-family: inherit;
  font-size: var(--fs-base);
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  outline: none;
  line-height: 1.5;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.input-box:focus {
  border-color: var(--primary);
}

.mention-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}

/* @ 成员 float 列表 */
.mention-float {
  position: absolute;
  left: 16px;
  bottom: 74px;
  z-index: 20;
  min-width: 220px;
  max-height: 240px;
  overflow-y: auto;
  padding: 6px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
}

.mention-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 10px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
}

.mention-item:hover {
  background: var(--card-2);
}

.mention-avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  background: var(--bg);
  font-size: 14px;
}

.mention-name {
  font-size: var(--fs-base);
}

/* float 弹出动画 */
.pop-enter-active {
  transition: opacity var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard);
}

.pop-enter-from {
  opacity: 0;
  transform: translateY(6px);
}

/* ============ 空态 ============ */
.chat-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--muted);
}

.chat-empty-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 72px;
  height: 72px;
  border-radius: 50%;
  background: var(--card-2);
  color: var(--muted);
}

.chat-empty-text {
  margin: 0;
  font-size: var(--fs-md);
  color: var(--text);
}

.chat-empty-hint {
  margin: 0;
  font-size: var(--fs-sm);
}

/* ============ 管理员悬浮操作栏 ============ */
.admin-actions {
  position: absolute;
  top: 52px;
  right: 16px;
  z-index: 15;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.14);
}

/* ============ Dialog 内容 ============ */
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
  border-radius: var(--radius-md);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.field-input:focus {
  border-color: var(--primary);
}

.field-input.taskarea {
  height: auto;
  padding: 8px 12px;
  resize: none;
  line-height: 1.5;
}

.member-checks {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

/* 成员列表 */
.member-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 320px;
  overflow-y: auto;
}

.member-row,
.addable-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border-radius: var(--radius-md);
  background: var(--card-2);
}

.member-avatar {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: 50%;
  background: var(--bg);
  font-size: 16px;
}

.member-info {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.member-name {
  font-size: var(--fs-base);
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.me-badge {
  margin-left: 4px;
  padding: 0 6px;
  font-size: var(--fs-xs);
  color: var(--primary);
  background: rgba(74, 126, 255, 0.12);
  border-radius: var(--radius-full);
}

.member-role {
  font-size: var(--fs-xs);
  color: var(--muted);
  flex-shrink: 0;
}

.member-empty {
  padding: 20px;
  text-align: center;
  font-size: var(--fs-sm);
  color: var(--muted);
}

.member-footer {
  margin-top: 12px;
}

.addable-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 320px;
  overflow-y: auto;
}

.dialog-delete-content {
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  padding: 4px 0;
}
</style>