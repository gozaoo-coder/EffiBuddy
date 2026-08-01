/**
 * HugeIcon —— Hugeicons 图标渲染器
 *
 * 直接消费 @hugeicons/core-free-icons 的图标数据（[tag, attrs][]），渲染为原生 SVG VNode。
 * 渲染逻辑对齐官方 @hugeicons/vue 的 HugeiconsIcon 组件：
 *   - camelCase 属性名转 kebab-case（如 strokeLinecap → stroke-linecap）
 *   - strokeWidth 覆盖：设置时统一写入 stroke-width 与 stroke="currentColor"
 *   - absoluteStrokeWidth：按尺寸缩放描边，使视觉线宽在不同 size 下保持一致
 *   - 颜色默认 currentColor，由父元素 CSS color 控制
 *
 * 不依赖 @hugeicons/vue（其 1.0.7 版本缺失 HugeiconsIcon.vue.d.ts 类型声明，
 * 会让 vue-tsc 严格构建报错），仅依赖官方图标数据包，类型完整、可 tree-shake。
 */
import { defineComponent, h, type PropType } from 'vue'

/** Hugeicons 图标元素属性（兼容官方 IconSvgObject 的可变/只读两个分支）。 */
type IconAttr = { readonly [key: string]: string | number }
/** 单个图标的元素数据：[标签名, 属性]。 */
type IconElement = readonly [string, IconAttr]
/** 规范化图标数据：只读元素数组。 */
export type NormalizedIcon = readonly IconElement[]

/** camelCase → kebab-case（linecap / Linejoin / Width 等 SVG 属性需 kebab 形式）。 */
const camelToKebab = (key: string): string =>
  key.replace(/([a-z0-9])([A-Z])/g, '$1-$2').toLowerCase()

export const HugeIcon = defineComponent({
  name: 'HugeIcon',
  inheritAttrs: false,
  props: {
    /** 图标数据（来自 @hugeicons/core-free-icons）。 */
    icon: { type: Array as unknown as PropType<NormalizedIcon>, required: true },
    /** 尺寸 px，默认 24。 */
    size: { type: [Number, String] as PropType<number | string>, default: 24 },
    /** 描边宽度覆盖；不传则沿用图标自带值（通常 1.5）。 */
    strokeWidth: { type: Number, default: undefined },
    /** 为 true 时按 size 反比缩放描边，保持视觉线宽恒定。 */
    absoluteStrokeWidth: { type: Boolean, default: false },
    /** 颜色，默认 currentColor（继承父级 color）。 */
    color: { type: String, default: 'currentColor' },
  },
  setup(props) {
    return () => {
      const raw = typeof props.size === 'string' ? parseInt(props.size, 10) : props.size
      const size = !isNaN(raw) && raw > 0 ? raw : 24

      const sw =
        props.strokeWidth === undefined
          ? undefined
          : props.absoluteStrokeWidth
            ? (props.strokeWidth * 24) / size
            : props.strokeWidth

      const nodes = props.icon.map((el, i) => {
        const [tag, attrs] = el
        const out: Record<string, string | number> = {}
        for (const [k, v] of Object.entries(attrs)) {
          // Hugeicons 数据里每个元素带一个内部 key 字段，不应作为 DOM 属性输出
          if (k === 'key') continue
          out[camelToKebab(k)] = v
        }
        if (sw !== undefined) {
          out['stroke-width'] = sw
          out['stroke'] = 'currentColor'
        }
        return h(tag, { ...out, key: i })
      })

      return h(
        'svg',
        {
          width: size,
          height: size,
          viewBox: '0 0 24 24',
          xmlns: 'http://www.w3.org/2000/svg',
          fill: 'none',
          color: props.color,
        },
        nodes,
      )
    }
  },
})
