<script setup lang="ts">
/**
 * ChatHome —— 新对话空状态首页
 *
 * 定位：替代旧版「中央品牌 logo」的纯装饰空态，改为「功能性引导」：
 *  1. 问候 + 已连接模型徽标（顶栏已去掉品牌，这里也不再堆 EffiBuddy 大字）
 *  2. 示例提示卡：点击一键发送（走 useChatSend.sendPrompt，与输入栏共用发送编排）
 *  3. 功能导航卡：点击直达 待办 / 插件 / 技能 / ClawHub / 自动化 / 语音转写 / 设置
 *
 * 动作解耦：功能导航经 appActions 全局动作中枢（App.vue 注册），
 * 空态卡片不需要知道面板开关的内部实现。
 *
 * 自适应（容器下的占位效果）：
 *  - home-inner 用 margin:auto 安全居中：容器够高时内容垂直居中；
 *    容器过矮时自动回退为顶部对齐并交给 overflow-y 滚动，杜绝「居中裁切」。
 *  - padding / gap / 字号用 clamp() 随容器线性缩放；
 *  - 卡片网格 minmax(min(x,100%)) 保证窄容器不横向溢出；
 *  - 矮窗口 / 窄窗口各有一组紧凑媒体查询。
 */
import { inject } from 'vue'
import { Icon } from '../basic'
import { useAppActions } from '../../composables/appActions'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const { sendPrompt } = store.send
const { activeModelInfo } = store.core
const { run: runAction } = useAppActions()

/** 示例提示：点击即一键发送 */
const prompts = [
  {
    icon: 'idea',
    title: '写方案',
    desc: '帮我写一份「AI 智能助手」实施方案，含功能规划与技术选型',
  },
  {
    icon: 'search',
    title: '深度研究',
    desc: '深度研究「AI Agent 最新进展」，整理一份结构化报告',
  },
  {
    icon: 'globe',
    title: '搭网站',
    desc: '帮我规划并搭建一个个人作品集网站，先给整体结构建议',
  },
  {
    icon: 'brain',
    title: '分析代码',
    desc: '帮我看一下当前项目的代码架构，指出可优化点',
  },
]

/** 功能导航：点击直达对应面板 / 页面（App.vue 注册的全局动作） */
const features = [
  { icon: 'book', title: '我的待办', desc: '任务清单', action: 'open-todo' },
  { icon: 'puzzle', title: '插件中心', desc: '安装与管理插件', action: 'open-plugin-panel' },
  { icon: 'tool', title: '技能库', desc: '技能 / 命令', action: 'open-skill-panel' },
  { icon: 'globe', title: 'ClawHub', desc: '技能与插件市场', action: 'open-clawhub' },
  { icon: 'alarm', title: '定时自动化', desc: '定时任务', action: 'open-automation' },
  { icon: 'mic', title: '语音转写', desc: '录音与转写', action: 'open-asr' },
  { icon: 'settings', title: '设置', desc: '模型与偏好', action: 'open-settings' },
]
</script>

<template>
  <div class="home-empty">
    <!-- 顶部光晕装饰 -->
    <div class="home-glow" aria-hidden="true"></div>

    <!-- home-inner：margin:auto 安全居中，内容超高时回退为顶部对齐 + 可滚动 -->
    <div class="home-inner">
      <!-- 问候区 -->
      <div class="home-hero">
        <div class="home-orb">
          <Icon name="spark" :size="30" />
        </div>
        <h1 class="home-title">你好，我是 <span class="home-title-accent">Effi</span></h1>
        <p class="home-subtitle">今天想让我帮你做点什么？</p>
        <div v-if="activeModelInfo?.name" class="home-model" :title="activeModelInfo.name">
          <span class="home-model-dot"></span>
          <span class="home-model-name">{{ activeModelInfo.name }}</span>
        </div>
      </div>

      <!-- 示例提示卡：一键发送 -->
      <div class="home-section">
        <div class="home-section-label">试试这样问</div>
        <div class="prompt-grid">
          <button
            v-for="p in prompts"
            :key="p.title"
            type="button"
            class="prompt-card"
            @click="sendPrompt(p.desc)"
          >
            <span class="prompt-icon"><Icon :name="p.icon" :size="18" /></span>
            <span class="prompt-body">
              <span class="prompt-title">{{ p.title }}</span>
              <span class="prompt-desc">{{ p.desc }}</span>
            </span>
            <span class="prompt-send"><Icon name="arrow-up" :size="14" /></span>
          </button>
        </div>
      </div>

      <!-- 功能导航卡 -->
      <div class="home-section">
        <div class="home-section-label">直达功能</div>
        <div class="feature-grid">
          <button
            v-for="f in features"
            :key="f.title"
            type="button"
            class="feature-card"
            @click="runAction(f.action)"
          >
            <span class="feature-icon"><Icon :name="f.icon" :size="20" /></span>
            <span class="feature-title">{{ f.title }}</span>
            <span class="feature-desc">{{ f.desc }}</span>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ===== 整体布局 ===== */
.home-empty {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  /* 横向留白随容器收缩；纵向上下对称、底部略大，给输入栏留呼吸感 */
  padding: clamp(20px, 3vh, 40px) clamp(16px, 3vw, 40px) clamp(32px, 6vh, 64px);
  overflow-x: hidden;
  overflow-y: auto; /* 极矮容器下允许滚动，而不是裁切 */
  text-align: center;
  scrollbar-width: thin;
}

/* home-inner：margin:auto 是「安全居中」的关键——
   容器够高时垂直水平居中；内容超高时 margin:auto 自动归零 → 顶部对齐 + 滚动，不再被裁切 */
.home-inner {
  margin: auto;
  width: 100%;
  max-width: 760px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: clamp(22px, 3.2vh, 34px);
}

/* 顶部光晕：主题色渐变氛围（随容器宽度收缩，避免小窗溢出） */
.home-glow {
  position: absolute;
  top: -180px;
  left: 50%;
  transform: translateX(-50%);
  width: min(560px, 92vw);
  height: 420px;
  border-radius: 50%;
  background: radial-gradient(
    closest-side,
    color-mix(in srgb, var(--primary) 16%, transparent),
    transparent 70%
  );
  pointer-events: none;
  z-index: 0;
}

/* ===== 问候区 ===== */
.home-hero {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: clamp(8px, 1.2vh, 12px);
  animation: home-in 0.4s ease both;
}

.home-orb {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: 22px;
  color: #fff;
  background: linear-gradient(
    135deg,
    var(--primary),
    color-mix(in srgb, var(--primary) 55%, var(--accent))
  );
  box-shadow: 0 10px 30px color-mix(in srgb, var(--primary) 35%, transparent);
}

.home-title {
  margin: 0;
  font-size: clamp(24px, 3.4vw, 30px);
  font-weight: 800;
  letter-spacing: -0.5px;
  color: var(--text);
}

.home-title-accent {
  background: linear-gradient(135deg, var(--primary), color-mix(in srgb, var(--primary) 55%, var(--accent)));
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
}

.home-subtitle {
  margin: 0;
  font-size: 14px;
  color: var(--muted);
}

/* 模型徽标：模型名过长时省略号截断，不撑破容器 */
.home-model {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  max-width: min(340px, 100%);
  margin-top: 2px;
  padding: 5px 12px;
  font-size: 12px;
  color: var(--text);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  overflow: hidden;
}

.home-model-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.home-model-dot {
  flex-shrink: 0;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--success);
  box-shadow: 0 0 6px var(--success);
}

/* ===== 区块 ===== */
.home-section {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: clamp(10px, 1.4vh, 14px);
  width: 100%;
  animation: home-in 0.45s ease both;
}
/* 错峰入场：第二、三块（卡片区）依次延迟浮现 */
.home-section:nth-of-type(2) {
  animation-delay: 0.05s;
}
.home-section:nth-of-type(3) {
  animation-delay: 0.1s;
}

.home-section-label {
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.5px;
  color: var(--muted);
}

/* ===== 示例提示卡 ===== */
.prompt-grid {
  display: grid;
  /* min(x, 100%)：窄容器下退化为单列且不横向溢出 */
  grid-template-columns: repeat(auto-fit, minmax(min(250px, 100%), 1fr));
  gap: 10px;
  width: 100%;
}

.prompt-card {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
  padding: 12px 14px;
  text-align: left;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: transform 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease,
    background 0.15s ease;
}

.prompt-card:hover {
  transform: translateY(-2px);
  border-color: color-mix(in srgb, var(--primary) 45%, var(--border));
  background: var(--card-2);
  box-shadow: 0 6px 18px color-mix(in srgb, var(--primary) 12%, transparent);
}

.prompt-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 36px;
  height: 36px;
  border-radius: 10px;
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 12%, transparent);
}

.prompt-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.prompt-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.prompt-desc {
  font-size: 12px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}

.prompt-send {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 24px;
  height: 24px;
  border-radius: 50%;
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 10%, transparent);
  opacity: 0;
  transform: translateX(-4px);
  transition: opacity 0.15s ease, transform 0.15s ease;
}

.prompt-card:hover .prompt-send {
  opacity: 1;
  transform: translateX(0);
}

/* ===== 功能导航卡 ===== */
.feature-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(148px, 100%), 1fr));
  gap: 10px;
  width: 100%;
}

.feature-card {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  min-width: 0;
  padding: 14px 16px;
  text-align: left;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: transform 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease,
    background 0.15s ease;
}

.feature-card:hover {
  transform: translateY(-2px);
  border-color: color-mix(in srgb, var(--primary) 40%, var(--border));
  background: var(--card-2);
  box-shadow: 0 6px 16px color-mix(in srgb, var(--primary) 10%, transparent);
}

.feature-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  margin-bottom: 4px;
  border-radius: 10px;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}

.feature-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.feature-desc {
  font-size: 11px;
  color: var(--muted);
}

/* ===== 轻量入场动画 ===== */
@keyframes home-in {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: none;
  }
}

@media (prefers-reduced-motion: reduce) {
  .home-hero,
  .home-section {
    animation: none;
  }
}

/* ===== 矮窗口：整体紧凑，避免上下拥挤 ===== */
@media (max-height: 620px) {
  .home-empty {
    padding-top: 14px;
    padding-bottom: 14px;
  }
  .home-inner {
    gap: 18px;
  }
  .home-orb {
    width: 50px;
    height: 50px;
    border-radius: 17px;
  }
  .home-title {
    font-size: 23px;
  }
  .home-hero {
    gap: 6px;
  }
  .home-section {
    gap: 8px;
  }
  .prompt-card {
    padding: 10px 12px;
  }
  .feature-card {
    padding: 12px 14px;
  }
}

/* ===== 窄窗口：收窄留白，避免卡片拥挤 ===== */
@media (max-width: 440px) {
  .home-empty {
    padding-left: 12px;
    padding-right: 12px;
  }
  .home-title {
    font-size: 24px;
  }
  .prompt-card {
    gap: 10px;
    padding: 10px 12px;
  }
  .prompt-icon {
    width: 32px;
    height: 32px;
    border-radius: 9px;
  }
  .feature-icon {
    width: 30px;
    height: 30px;
  }
}
</style>
