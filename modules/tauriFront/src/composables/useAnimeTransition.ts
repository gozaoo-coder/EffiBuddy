/**
 * useAnimeTransition —— 用 anime.js v4 驱动 Vue <Transition> 的 JS 钩子
 *
 * 为什么需要它：
 * Vue 原生 <Transition> 依赖 CSS class（.xxx-enter-active 等），但 anime.js v4
 * 提供了更精细的缓动（out(3)、inOut(2.5) 等弹性曲线）、stagger、timeline 编排。
 * 通过 JS 钩子 + anime.js，可以获得比纯 CSS 过渡更流畅、可控的动效，并且
 * 解决 Vue v-if 立即卸载 DOM 导致 leaveTo 动画丢失的问题。
 *
 * 用法：
 *   const { onEnter, onLeave } = useAnimeTransition({
 *     enter: { opacity: [0, 1], transform: ['scale(.9)', 'scale(1)'], duration: 280 },
 *     leave: { opacity: [1, 0], transform: ['scale(1)', 'scale(.9)'], duration: 220 },
 *   })
 *   <Transition :css="false" @enter="onEnter" @leave="onLeave">...</Transition>
 *
 * Vue 钩子签名约定：
 *   enter(el, done) —— 元素插入 DOM 后触发，调用 done() 通知 Vue 完成
 *   leave(el, done) —— 元素即将移除前触发，调用 done() 通知 Vue 可移除
 */
import { animate, type AnimationParams } from 'animejs'

export interface AnimeTransitionOptions {
  /** 进入动画参数（anime.js v4） */
  enter?:
    | AnimationParams
    | ((el: Element, done: () => void) => void | ReturnType<typeof animate>)
  /** 离开动画参数 */
  leave?:
    | AnimationParams
    | ((el: Element, done: () => void) => void | ReturnType<typeof animate>)
  /** 进入前重置样式（避免首帧闪烁） */
  beforeEnter?: (el: Element) => void
  /** 离开前预处理 */
  beforeLeave?: (el: Element) => void
}

/**
 * 将 anime.js v4 动画参数转换为 Vue Transition 钩子。
 * 返回可直接绑定到 <Transition :css="false" @enter @leave @before-enter @before-leave> 的函数。
 */
export function useAnimeTransition(options: AnimeTransitionOptions) {
  function onBeforeEnter(el: Element) {
    if (options.beforeEnter) {
      options.beforeEnter(el)
      return
    }
    // 默认在进入前确保元素可见，避免残留 opacity:0
    const htmlEl = el as HTMLElement
    if (htmlEl.style.opacity === '0') htmlEl.style.opacity = ''
  }

  function onEnter(el: Element, done: () => void) {
    const enter = options.enter
    if (!enter) {
      done()
      return
    }
    if (typeof enter === 'function') {
      enter(el, done)
      return
    }
    // 动画完成后调用 done()，让 Vue 移除 transition 状态
    // anime.js v4: animate(targets, params)
    animate(el, {
      ...enter,
      onComplete: () => {
        // 清理内联 transform，避免影响后续布局
        const htmlEl = el as HTMLElement
        htmlEl.style.transform = ''
        htmlEl.style.opacity = ''
        done()
      },
    })
  }

  function onBeforeLeave(el: Element) {
    if (options.beforeLeave) {
      options.beforeLeave(el)
    }
  }

  function onLeave(el: Element, done: () => void) {
    const leave = options.leave
    if (!leave) {
      done()
      return
    }
    if (typeof leave === 'function') {
      leave(el, done)
      return
    }
    animate(el, {
      ...leave,
      onComplete: () => {
        done()
      },
    })
  }

  return { onBeforeEnter, onEnter, onBeforeLeave, onLeave }
}

/**
 * 常用浮层动画预设：淡入 + 缩放（适合 Dialog / Menu / Popup / Dropdown 面板）
 */
export const fadeScaleTransition = {
  enter: {
    opacity: [0, 1],
    transform: ['scale(.92)', 'scale(1)'],
    duration: 260,
    ease: 'out(3)',
  } as AnimationParams,
  leave: {
    opacity: [1, 0],
    transform: ['scale(1)', 'scale(.92)'],
    duration: 200,
    ease: 'out(3)',
  } as AnimationParams,
}

/**
 * 从指定方向滑入/滑出（适合 BindSheet / Picker sheet）
 */
export function slideTransition(
  direction: 'up' | 'down' | 'left' | 'right',
  distance = 100,
) {
  const axis = direction === 'up' || direction === 'down' ? 'translateY' : 'translateX'
  const sign = direction === 'up' || direction === 'left' ? '-' : ''
  const enterFrom = `${axis}(${sign}${distance}px)`
  const leaveTo = `${axis}(${sign}${distance}px)`
  return {
    enter: {
      opacity: [0, 1],
      transform: [enterFrom, `${axis}(0px)`],
      duration: 300,
      ease: 'out(3)',
    } as AnimationParams,
    leave: {
      opacity: [1, 0],
      transform: [`${axis}(0px)`, leaveTo],
      duration: 240,
      ease: 'inOut(2)',
    } as AnimationParams,
  }
}
