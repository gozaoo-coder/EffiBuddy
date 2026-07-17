<script setup lang="ts">
import { ref } from 'vue'
import {
  ToastHost,
  SnackbarHost,
  Popup,
  Button,
  ToggleButton,
  IconButton,
  Dropdown,
  Switch,
  Radio,
  RadioGroup,
  Slider,
  SegmentedButton,
  Picker,
  Chips,
  Menu,
  Dialog,
  BindSheet,
  Icon,
  useToast,
  useSnackbar,
} from './basic'
import type { MenuItemOption } from './basic'

// ===== Toast/Snackbar =====
const { toast } = useToast()
const { snackbar } = useSnackbar()

function showToast(type: 'info' | 'success' | 'warn' | 'error') {
  toast({
    content: { info: '信息提示', success: '操作成功', warn: '警告信息', error: '出错了' }[type],
    type,
    duration: 3000,
  })
}

function showSnackbar(mode: 'timed' | 'persistent') {
  snackbar({
    content: mode === 'timed' ? '已删除 3 项' : '请先登录以继续操作',
    mode,
    duration: 5000,
    action: mode === 'timed' ? { text: '撤销', onClick: () => toast({ content: '已撤销', type: 'success' }) } : undefined,
  })
}

// ===== Toggle =====
const toggleVal = ref(false)

// ===== Dropdown =====
const dropdownVal = ref('apple')
const dropdownOptions = [
  { label: '苹果', value: 'apple', icon: '🍎' },
  { label: '香蕉', value: 'banana', icon: '🍌' },
  { label: '橙子', value: 'orange', icon: '🍊' },
  { label: '葡萄', value: 'grape', icon: '🍇' },
  { label: '西瓜', value: 'watermelon', icon: '🍉' },
]

// ===== Switch =====
const switchVal = ref(true)

// ===== Radio =====
const radioVal = ref('a')

// ===== Slider =====
const sliderVal = ref(40)

// ===== Segmented =====
const segVal = ref('day')

// ===== Picker =====
const pickerVal = ref('mon')

// ===== Chips =====
const chipSelected = ref(false)
const chips = ref([
  { label: '张三', image: '', removable: true },
  { label: '李四', removable: true },
  { label: '王五', removable: true },
])

function removeChip(i: number) {
  chips.value.splice(i, 1)
}

// ===== Menu =====
const menuVisible = ref(false)
const menuItems: MenuItemOption[] = [
  { key: 'new', label: '新建', icon: 'plus' },
  { key: 'open', label: '打开', icon: 'folder' },
  { key: 'save', label: '保存', icon: 'cloud', divided: true },
  { key: 'export', label: '导出', icon: 'external-link', children: [
    { key: 'pdf', label: 'PDF' },
    { key: 'word', label: 'Word' },
    { key: 'html', label: 'HTML' },
  ]},
  { key: 'delete', label: '删除', icon: 'delete', danger: true, divided: true },
]

function onMenuSelect(item: MenuItemOption) {
  toast({ content: `点击了 ${item.label}`, type: 'info' })
  menuVisible.value = false
}

// ===== Dialog =====
const dialogVisible = ref(false)
const dangerDialogVisible = ref(false)

// ===== BindSheet =====
const sheetVisible = ref(false)
const sheetSide = ref<'bottom' | 'right'>('bottom')
</script>

<template>
  <div class="demo-page">
    <h1 class="demo-title">EffiSuite 基础组件库</h1>
    <p class="demo-desc">参考 HarmonyOS NEXT 设计规范，统一圆角、高度、动效</p>

    <!-- Toast / Snackbar -->
    <section class="demo-section">
      <h2>即时反馈 Toast</h2>
      <div class="demo-row">
        <Button size="sm" @click="showToast('info')">信息</Button>
        <Button size="sm" variant="primary" @click="showToast('success')">成功</Button>
        <Button size="sm" variant="danger" @click="showToast('error')">错误</Button>
        <Button size="sm" variant="normal" @click="showToast('warn')">警告</Button>
      </div>

      <h2>即时操作 Snackbar</h2>
      <div class="demo-row">
        <Button size="sm" @click="showSnackbar('timed')">定时关闭（带撤销）</Button>
        <Button size="sm" variant="normal" @click="showSnackbar('persistent')">常驻模式</Button>
      </div>
    </section>

    <!-- Popup -->
    <section class="demo-section">
      <h2>气泡提示 Popup</h2>
      <div class="demo-row">
        <Popup message="这是简单的消息提示" placement="top">
          <Button size="sm">Message</Button>
        </Popup>
        <Popup message="带关闭按钮的提示" show-close placement="top">
          <Button size="sm">Message + Close</Button>
        </Popup>
        <Popup title="标题" message="带标题和关闭的提示" show-close placement="top">
          <Button size="sm">Message + Close + Title</Button>
        </Popup>
        <Popup
          title="确认"
          message="是否删除该项？"
          show-close
          :button="{ text: '删除', onClick: () => showToast('success') }"
          placement="top"
        >
          <Button size="sm">Message + Close + Title + Button</Button>
        </Popup>
        <Popup message="带图标的提示" icon="💡" placement="bottom">
          <Button size="sm">Message + Icon</Button>
        </Popup>
      </div>
    </section>

    <!-- Button 组 -->
    <section class="demo-section">
      <h2>按钮 Button</h2>
      <div class="demo-row">
        <Button variant="primary">强调按钮</Button>
        <Button variant="normal">普通按钮</Button>
        <Button variant="text">文字按钮</Button>
        <Button variant="danger">危险按钮</Button>
      </div>
      <div class="demo-row">
        <Button size="sm" variant="primary">小</Button>
        <Button size="md" variant="primary">中</Button>
        <Button size="lg" variant="primary">大</Button>
        <Button :loading="true" variant="primary">加载中</Button>
        <Button :disabled="true" variant="primary">禁用</Button>
      </div>
      <div class="demo-row">
        <IconButton container><Icon name="settings" :size="20" /></IconButton>
        <IconButton container variant="primary"><Icon name="search" :size="20" /></IconButton>
        <IconButton container variant="danger"><Icon name="delete" :size="20" /></IconButton>
        <ToggleButton v-model="toggleVal" active-text="已启用" inactive-text="已禁用" />
      </div>
    </section>

    <!-- Dropdown -->
    <section class="demo-section">
      <h2>下拉按钮 Dropdown</h2>
      <div class="demo-row">
        <Dropdown v-model="dropdownVal" :options="dropdownOptions" placeholder="选择水果" style="width: 200px" />
        <Dropdown v-model="dropdownVal" :options="dropdownOptions" searchable placeholder="搜索水果" style="width: 240px" />
      </div>
      <p class="demo-value">当前选择：{{ dropdownVal }}</p>
    </section>

    <!-- 选择类 -->
    <section class="demo-section">
      <h2>开关 Switch</h2>
      <div class="demo-row">
        <Switch v-model="switchVal" size="sm" />
        <Switch v-model="switchVal" size="md" />
        <Switch v-model="switchVal" size="lg" />
        <span class="demo-value">{{ switchVal ? '开' : '关' }}</span>
      </div>

      <h2>单选 Radio</h2>
      <RadioGroup v-model="radioVal">
        <Radio value="a" label="选项 A" />
        <Radio value="b" label="选项 B" />
        <Radio value="c" label="选项 C" />
      </RadioGroup>
      <p class="demo-value">当前选择：{{ radioVal }}</p>

      <h2>滑动条 Slider</h2>
      <div class="demo-row" style="width: 320px">
        <Slider v-model="sliderVal" :min="0" :max="100" :step="1" show-value />
      </div>

      <h2>分段按钮 SegmentedButton</h2>
      <SegmentedButton
        v-model="segVal"
        :options="[
          { label: '日', value: 'day' },
          { label: '周', value: 'week' },
          { label: '月', value: 'month' },
          { label: '年', value: 'year' },
        ]"
      />

      <h2>选择器 Picker</h2>
      <Picker
        v-model="pickerVal"
        :options="[
          { label: '星期一', value: 'mon' },
          { label: '星期二', value: 'tue' },
          { label: '星期三', value: 'wed' },
          { label: '星期四', value: 'thu' },
          { label: '星期五', value: 'fri' },
        ]"
        title="选择星期"
        style="width: 200px"
      />
      <p class="demo-value">当前选择：{{ pickerVal }}</p>
    </section>

    <!-- Chips -->
    <section class="demo-section">
      <h2>操作块 Chips</h2>
      <div class="demo-row">
        <Chips
          v-for="(c, i) in chips"
          :key="i"
          :label="c.label"
          :removable="c.removable"
          :selected="chipSelected"
          @remove="removeChip(i)"
          @click="chipSelected = !chipSelected"
        />
        <Chips label="带图标" icon="📧" />
        <Chips label="不可删除" />
      </div>
    </section>

    <!-- Menu / Dialog / BindSheet -->
    <section class="demo-section">
      <h2>菜单 Menu</h2>
      <div class="demo-row">
        <Button variant="normal" @click="menuVisible = !menuVisible">打开菜单</Button>
        <Menu v-model:visible="menuVisible" :items="menuItems" placement="bottom-start" @select="onMenuSelect">
          <Button variant="normal">带子菜单</Button>
        </Menu>
      </div>

      <h2>弹出框 Dialog</h2>
      <div class="demo-row">
        <Button variant="primary" @click="dialogVisible = true">普通对话框</Button>
        <Button variant="danger" @click="dangerDialogVisible = true">危险对话框</Button>
      </div>

      <h2>半模态面板 BindSheet</h2>
      <div class="demo-row">
        <Button variant="normal" @click="sheetSide = 'bottom'; sheetVisible = true">底部滑入</Button>
        <Button variant="normal" @click="sheetSide = 'right'; sheetVisible = true">右侧滑入</Button>
      </div>
    </section>

    <!-- 全局 Host -->
    <ToastHost />
    <SnackbarHost />

    <!-- Dialog -->
    <Dialog
      v-model:visible="dialogVisible"
      title="提示"
      content="这是一个普通对话框，用于确认用户操作。"
    />
    <Dialog
      v-model:visible="dangerDialogVisible"
      title="危险操作"
      content="确定要删除该会话吗？此操作不可撤销。"
      :danger="true"
      confirm-text="删除"
    />

    <!-- BindSheet -->
    <BindSheet
      v-model:visible="sheetVisible"
      :side="sheetSide"
      title="半模态面板"
      :height="sheetSide === 'bottom' ? '50vh' : undefined"
      :width="sheetSide === 'right' ? '480px' : undefined"
    >
      <div style="padding: 16px; color: var(--text)">
        <p>这是半模态面板的内容区域，可放置任意内容。</p>
        <p>从{{ sheetSide === 'bottom' ? '底部' : '右侧' }}滑入。</p>
      </div>
    </BindSheet>
  </div>
</template>

<style scoped>
.demo-page {
  padding: 24px 32px;
  max-width: 960px;
  margin: 0 auto;
  color: var(--text);
}

.demo-title {
  font-size: 24px;
  font-weight: 600;
  margin: 0 0 8px;
}

.demo-desc {
  color: var(--muted);
  margin: 0 0 24px;
}

.demo-section {
  margin-bottom: 32px;
  padding: 16px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.demo-section h2 {
  font-size: 15px;
  font-weight: 600;
  margin: 0 0 12px;
  color: var(--text);
}

.demo-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
}

.demo-value {
  font-size: 12px;
  color: var(--muted);
  margin: 4px 0;
}
</style>
