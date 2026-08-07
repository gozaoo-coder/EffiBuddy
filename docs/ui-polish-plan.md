# EffiSuite UI Polish Plan

> 基于 emilkowalski 设计工程方法论（`~/.agents/skills/emil-design-eng`）与动画评审标准（`~/.agents/skills/review-animations/STANDARDS.md`）制定的全程序用户界面优化方案。

## 一、设计原则（来自 emil-design-eng）

1. **看不见的细节叠加成惊艳** —— 大多数细节用户不会刻意注意，这正是目的。
2. **审美是杠杆** —— 人按整体体验选工具，默认值与动画是真正的差异化。
3. **每个动画必须有目的** —— "看起来酷"但用户高频看到 → 不动画。

## 二、动效决策框架

| 使用频率 | 决策 |
| --- | --- |
| 100+ 次/天（快捷键、面板切换） | 永不动画 |
| 数十次/天（hover、列表导航） | 移除或大幅削减 |
| 偶尔（模态、抽屉、toast） | 标准动画 |
| 罕见/首次（引导、庆祝） | 可加惊喜 |

- 进入/退出 → `ease-out`（起步快，响应感强）
- 屏上移动/变形 → `ease-in-out`
- hover/颜色变化 → `ease`
- **UI 动画统一 < 300ms**；按压反馈 100-160ms

## 三、设计 Token 增强（main.css `:root`）

```css
/* emil 推荐强曲线（补齐现有 Material 系曲线） */
--ease-out: cubic-bezier(0.23, 1, 0.32, 1);        /* 强 ease-out：UI 交互 */
--ease-in-out: cubic-bezier(0.77, 0, 0.175, 1);    /* 强 ease-in-out：屏上移动 */
--ease-drawer: cubic-bezier(0.32, 0.72, 0, 1);     /* 抽屉（iOS 式） */
--duration-press: 0.16s;                           /* 按压反馈 */
```

## 四、全局修复清单（评审格式）

| Before | After | Why |
| --- | --- | --- |
| `transition: all 0.15s`（27 处） | `transition: transform 0.15s, opacity 0.15s, ...` | 只动画 transform/opacity（GPU 合成），避免全部属性重算 |
| `transform: scale(0)` 入场（useLayout/AgentPool/AskUserDialog/Menu dot 等） | `transform: scale(0.95); opacity: 0` | 现实中无物凭空出现；从 0.95 起更自然 |
| `ease-in` 起步 | `ease-out` / 自定义强曲线 | ease-in 起步慢，让界面显得迟钝 |
| 常规 UI 400ms | ≤300ms（150-250ms） | 180ms 下拉比 400ms 更跟手 |
| 无 `:active` 反馈 | `transform: scale(0.97)`（160ms ease-out） | 按钮必须对按压有回应 |
| popover 从中心缩放 | `transform-origin` 跟随触发点 | 弹出层应从触发器生长（模态除外） |

## 五、组件级优化

| 组件 | 动作 |
| --- | --- |
| `Button.vue` / `IconButton.vue` | 保留 anime 按压；统一 120-160ms、scale 0.97 |
| `Menu.vue` / `Popup.vue` / `Dialog.vue` | 入场 ease-out 150-250ms；transform-origin 跟随触发点（模态保持 center） |
| `TabBar.vue` | 指示条过渡平滑（transform 驱动） |
| `ChatMessageList.vue` / `MessageBubble.vue` | 消息入场 stagger 30-80ms + fadeInUp 300ms ease-out |
| `useLayout.ts`（面板切换） | `leaveTo scale(0)` → `scale(0.95) + opacity` |
| `AgentPool*` / `AskUserDialog.vue` | keyframe `scale(0)` → `scale(0.9-0.95) + opacity` |

## 六、无障碍

```css
@media (prefers-reduced-motion: reduce) {
  /* 保留透明度/颜色过渡，移除位移/缩放 */
}
@media (hover: hover) and (pointer: fine) {
  /* hover 动画仅精确指针设备触发 */
}
```

## 七、执行分工

- **Agent A（样式清理）**：只改 `styles/main.css` —— transition:all 拆解、scale(0) 修复、prefers-reduced-motion 降级、hover 媒体查询。
- **Agent B（组件动效）**：只改 `.vue` 组件与 `useLayout.ts` —— 组件级入场/按压/指示条/消息 stagger。
- 文件集互不重叠，可并行；完成后统一验证（vue-tsc + vite build + cargo build）。
