<script setup lang="ts">
/**
 * AskUserDialog —— "AI 询问用户"对话框
 *
 * 由后端 ask_user 工具触发:agent 在执行任务时遇到需要用户确认的问题,
 * 通过 BusEvent::AskUser 推送到前端,弹出此对话框收集用户选择。
 *
 * - 单选(multi_select=false): 点击选项卡片立即推进到下一题
 * - 多选(multi_select=true): 点击切换选中,需点击底部"提交"按钮才推进
 *
 * 全部问题答完后,答案被格式化为用户消息发送给后端(走标准 send_message_stream),
 * 在聊天列表中显示为一条用户气泡(展示简要摘要)。
 *
 * 动画管线:
 *  - 对话框进出场:由 Dialog.vue 的 useAnimeTransition 处理(scale + fade)
 *  - 问题切换:Vue <Transition mode="out-in"> + anime.js,旧问题 fade+slide-out-left,
 *    新问题 fade+slide-in-from-right
 *  - 选项卡片 hover:CSS transform/border/shadow 过渡
 *  - 选项卡片 click:anime.js scale 脉冲(1→0.96→1)
 *  - 选项卡片选中态:CSS border/background 过渡 + 勾选图标弹性缩放
 *  - 进度条:CSS width 过渡(cubic-bezier 弹性曲线)
 */
import { computed, inject, ref } from 'vue'
import { animate } from 'animejs'
import { Dialog, Icon } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const {
  visible,
  currentQuestions,
  currentQuestionIndex,
  selectedOptions,
  submitting,
  selectOption,
  submitCurrent,
  dismiss,
} = store.askUser

// ---------- 派生状态 ----------
const currentQuestion = computed(
  () => currentQuestions.value[currentQuestionIndex.value] ?? null,
)
const isMultiSelect = computed(() => !!currentQuestion.value?.multi_select)
const totalQuestions = computed(() => currentQuestions.value.length)
const title = computed(() => currentQuestion.value?.header || 'AI 想确认一些事')
const canSubmit = computed(() => selectedOptions.value.size > 0 && !submitting.value)

// ---------- Dialog visible 代理 ----------
// Dialog 任何关闭路径(跳过/×/ESC/遮罩)都走 update:visible(false) → dismiss()
// 注意:finalizeAndSend 外部赋值 visible.value=false 不会触发 update:visible,
// 因此 dismiss 不会被调用,collectedAnswers 得以保留供 build* 使用。
function onVisibleUpdate(v: boolean) {
  if (!v) dismiss()
}

// ---------- 选项交互 ----------
function isOptionSelected(idx: number): boolean {
  return selectedOptions.value.has(idx)
}

function onOptionClick(ev: MouseEvent, idx: number) {
  // 点击脉冲:scale 缩放反馈
  const el = ev.currentTarget as HTMLElement
  animate(el, {
    scale: [1, 0.96, 1],
    duration: 240,
    ease: 'inOut(2)',
    onComplete: () => {
      // 清理内联 transform,避免影响后续 CSS hover/transition
      el.style.transform = ''
    },
  })
  selectOption(idx)
}

// ---------- 提交按钮(多选) ----------
const submitBtnRef = ref<HTMLButtonElement | null>(null)

function onSubmitClick() {
  if (!canSubmit.value) return
  if (submitBtnRef.value) {
    animate(submitBtnRef.value, {
      scale: [1, 0.97, 1],
      duration: 180,
      ease: 'out(3)',
      onComplete: () => {
        if (submitBtnRef.value) submitBtnRef.value.style.transform = ''
      },
    })
  }
  submitCurrent()
}

// ---------- 问题切换动画 ----------
// currentQuestionIndex 变化时:旧问题 fade+slide-out-left,新问题 fade+slide-in-from-right
// 使用 mode="out-in" 确保旧问题离场完成后新问题才入场,避免重叠
function onQuestionEnter(el: Element, done: () => void) {
  animate(el, {
    opacity: [0, 1],
    transform: ['translateX(24px)', 'translateX(0px)'],
    duration: 300,
    ease: 'out(3)',
    onComplete: () => {
      const htmlEl = el as HTMLElement
      htmlEl.style.transform = ''
      htmlEl.style.opacity = ''
      done()
    },
  })
}

function onQuestionLeave(el: Element, done: () => void) {
  animate(el, {
    opacity: [1, 0],
    transform: ['translateX(0px)', 'translateX(-24px)'],
    duration: 200,
    ease: 'inOut(2)',
    onComplete: () => {
      done()
    },
  })
}
</script>

<template>
  <Dialog
    :visible="visible"
    @update:visible="onVisibleUpdate"
    :title="title"
    cancel-text="跳过"
    :show-confirm="false"
    width="480px"
  >
    <div class="ask-user-body">
      <!-- 进度条(多问题时显示) -->
      <div v-if="totalQuestions > 1" class="ask-progress">
        <div class="ask-progress-text">
          第 {{ currentQuestionIndex + 1 }} / {{ totalQuestions }} 个问题
        </div>
        <div class="ask-progress-track">
          <div
            class="ask-progress-fill"
            :style="{
              width: `${((currentQuestionIndex + 1) / totalQuestions) * 100}%`,
            }"
          ></div>
        </div>
      </div>

      <!-- 问题内容(切换时滑入/滑出) -->
      <Transition :css="false" mode="out-in" appear
        @enter="onQuestionEnter" @leave="onQuestionLeave">
        <div :key="currentQuestionIndex" class="question-block">
          <div class="question-text">{{ currentQuestion?.question }}</div>

          <!-- 选项卡片 -->
          <div class="options-list">
            <button
              v-for="(opt, idx) in (currentQuestion?.options ?? [])"
              :key="idx"
              type="button"
              class="option-card"
              :class="{ 'option-card--selected': isOptionSelected(idx) }"
              @click="onOptionClick($event, idx)"
            >
              <div class="option-card-main">
                <div class="option-label">{{ opt.label }}</div>
                <div v-if="opt.description" class="option-desc">{{ opt.description }}</div>
              </div>
              <div class="option-card-check">
                <Icon name="check" :size="14" />
              </div>
            </button>
          </div>
        </div>
      </Transition>

      <!-- 多选提交按钮 -->
      <div v-if="isMultiSelect" class="ask-submit-row">
        <button
          ref="submitBtnRef"
          type="button"
          class="ask-submit-btn"
          :class="{ 'ask-submit-btn--disabled': !canSubmit }"
          :disabled="!canSubmit"
          @click="onSubmitClick"
        >
          提交
          <span v-if="selectedOptions.size > 0" class="ask-submit-count">
            ({{ selectedOptions.size }})
          </span>
        </button>
      </div>
    </div>
  </Dialog>
</template>

<style scoped>
.ask-user-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 4px 0 0;
  min-height: 80px;
}

/* ---------- 进度条 ---------- */
.ask-progress {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.ask-progress-text {
  font-size: 12px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
  letter-spacing: 0.02em;
}

.ask-progress-track {
  height: 3px;
  background: var(--bg-2);
  border-radius: var(--radius-full);
  overflow: hidden;
  position: relative;
}

.ask-progress-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--primary-dim, var(--primary)), var(--primary));
  border-radius: var(--radius-full);
  transition: width 0.45s cubic-bezier(0.22, 1, 0.36, 1);
  will-change: width;
}

/* ---------- 问题内容 ---------- */
.question-block {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.question-text {
  font-size: 16px;
  line-height: 1.55;
  color: var(--text);
  font-weight: 500;
  word-break: break-word;
}

/* ---------- 选项卡片 ---------- */
.options-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.option-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  background: var(--card);
  border: 1.5px solid var(--border);
  border-radius: var(--radius-lg);
  cursor: pointer;
  text-align: left;
  font-family: inherit;
  transition:
    transform 0.2s cubic-bezier(0.22, 1, 0.36, 1),
    border-color 0.18s ease,
    background 0.18s ease,
    box-shadow 0.18s ease;
  will-change: transform;
}

.option-card:hover {
  transform: translateY(-2px);
  border-color: color-mix(in srgb, var(--primary) 50%, var(--border));
  box-shadow: 0 4px 14px color-mix(in srgb, var(--primary) 10%, transparent);
}

.option-card:active {
  transform: translateY(0);
}

.option-card--selected {
  border-color: var(--primary);
  background: color-mix(in srgb, var(--primary) 10%, var(--card));
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 14%, transparent);
}

.option-card--selected:hover {
  transform: translateY(-2px);
  border-color: var(--primary);
  background: color-mix(in srgb, var(--primary) 14%, var(--card));
  box-shadow: 0 4px 14px color-mix(in srgb, var(--primary) 18%, transparent);
}

.option-card-main {
  flex: 1;
  min-width: 0;
}

.option-label {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  line-height: 1.4;
}

.option-desc {
  margin-top: 2px;
  font-size: 12px;
  color: var(--muted);
  line-height: 1.45;
  word-break: break-word;
}

/* 选中态勾选图标:从 0.5 缩放到 1,带弹性曲线 */
.option-card-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--primary);
  color: #fff;
  opacity: 0;
  transform: scale(0.5);
  transition:
    opacity 0.2s ease,
    transform 0.28s cubic-bezier(0.34, 1.56, 0.64, 1);
}

.option-card--selected .option-card-check {
  opacity: 1;
  transform: scale(1);
}

/* ---------- 提交按钮(多选) ---------- */
.ask-submit-row {
  display: flex;
  justify-content: flex-end;
  padding-top: 2px;
}

.ask-submit-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 8px 22px;
  font-size: 14px;
  font-weight: 600;
  font-family: inherit;
  color: #fff;
  background: var(--primary);
  border: none;
  border-radius: var(--radius-full);
  cursor: pointer;
  transition:
    opacity 0.15s ease,
    box-shadow 0.18s ease,
    transform 0.15s ease;
  box-shadow: 0 2px 10px color-mix(in srgb, var(--primary) 30%, transparent);
}

.ask-submit-btn:hover {
  box-shadow: 0 4px 14px color-mix(in srgb, var(--primary) 42%, transparent);
}

.ask-submit-btn--disabled {
  opacity: 0.5;
  cursor: not-allowed;
  box-shadow: none;
}

.ask-submit-btn--disabled:hover {
  box-shadow: none;
}

.ask-submit-count {
  font-variant-numeric: tabular-nums;
  opacity: 0.85;
  font-weight: 500;
}
</style>
