<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { use } from 'echarts/core'
import { CanvasRenderer } from 'echarts/renderers'
import { BarChart } from 'echarts/charts'
import { GridComponent, TooltipComponent, LegendComponent, DataZoomComponent } from 'echarts/components'
import VChart from 'vue-echarts'
import { useToast } from '../composables/useToast'
import SettingCard from '../components/SettingCard.vue'

use([CanvasRenderer, BarChart, GridComponent, TooltipComponent, LegendComponent, DataZoomComponent])

const toast = useToast()

interface UsageBucket {
  input: number
  output: number
  cache_read: number
  cache_write: number
  reasoning: number
  requests: number
}

interface DayStats {
  date: string
  zcode: UsageBucket
  claude: UsageBucket
  models: ModelStats[]
}

interface ModelStats {
  model: string
  source: string
  provider: string
  usage: UsageBucket
}

interface TokenStatsResult {
  zcode_detected: boolean
  claude_detected: boolean
  granularity: string
  zcode_sessions: number
  claude_sessions: number
  totals_zcode: UsageBucket
  totals_claude: UsageBucket
  by_day: DayStats[]
  by_model: ModelStats[]
}

type Source = 'all' | 'zcode' | 'claude'
type RangeKey = 'h8' | 'h12' | 'h24' | 'd7' | 'd90' | 'all' | 'custom'
type BucketKey = 'input' | 'output' | 'cache_read' | 'cache_write' | 'reasoning'

const AXIS_COLOR = '#64748b'

// 图表分段（堆叠顺序 = 数组顺序，自下而上）
const CATEGORIES: { key: BucketKey; label: string; color: string }[] = [
  { key: 'input', label: 'Input', color: '#3859ff' },
  { key: 'output', label: 'Output', color: '#8b5cf6' },
  { key: 'cache_read', label: '缓存读取', color: '#14b8a6' },
  { key: 'cache_write', label: '缓存写入', color: '#f59e0b' },
  { key: 'reasoning', label: 'Reasoning', color: '#ec4899' },
]

const source = ref<Source>('all')
const rangeKey = ref<RangeKey>('h12')
const customDays = ref(7)
const result = ref<TokenStatsResult | null>(null)
const loading = ref(false)

// 模型多选筛选（key = source|provider|model，空数组表示不过滤）
const selectedModels = ref<string[]>([])
const modelMenuOpen = ref(false)
const modelMenuRef = ref<HTMLElement | null>(null)
const menuStyle = ref<Record<string, string>>({})

/** 打开时测量按钮位置，自动选择左/右对齐与上/下弹出，避免被窗口边缘遮挡 */
function positionMenu() {
  const btn = modelMenuRef.value?.querySelector('.model-toggle')
  if (!(btn instanceof HTMLElement)) return
  const rect = btn.getBoundingClientRect()
  const MENU_W = 320
  const MENU_H = 280
  const vw = window.innerWidth
  const vh = window.innerHeight
  const style: Record<string, string> = {}

  // 水平：右侧空间够则左对齐，否则右对齐；两侧都不够时向较大一侧对齐并压缩宽度
  const spaceRight = vw - rect.left
  const spaceLeft = rect.right
  if (spaceRight >= MENU_W) {
    style.left = '0'
  } else if (spaceLeft >= MENU_W) {
    style.right = '0'
  } else if (spaceRight >= spaceLeft) {
    style.left = '0'
    style.maxWidth = `${Math.max(200, spaceRight - 8)}px`
  } else {
    style.right = '0'
    style.maxWidth = `${Math.max(200, spaceLeft - 8)}px`
  }

  // 垂直：下方空间不足时向上弹出
  style[rect.bottom + MENU_H > vh - 8 ? 'bottom' : 'top'] = 'calc(100% + 6px)'
  menuStyle.value = style
}

function toggleModelMenu() {
  modelMenuOpen.value = !modelMenuOpen.value
  if (modelMenuOpen.value) positionMenu()
}

const sourceOptions: { value: Source; label: string }[] = [
  { value: 'all', label: '全部' },
  { value: 'zcode', label: 'Zcode' },
  { value: 'claude', label: 'Claude Code' },
]

const rangeOptions: { value: RangeKey; label: string }[] = [
  { value: 'h8', label: '8 小时' },
  { value: 'h12', label: '12 小时' },
  { value: 'h24', label: '24 小时' },
  { value: 'd7', label: '7 天' },
  { value: 'd90', label: '90 天' },
  { value: 'all', label: '全部' },
  { value: 'custom', label: '自定义' },
]

function emptyBucket(): UsageBucket {
  return { input: 0, output: 0, cache_read: 0, cache_write: 0, reasoning: 0, requests: 0 }
}

function addBuckets(a: UsageBucket, b: UsageBucket): UsageBucket {
  return {
    input: a.input + b.input,
    output: a.output + b.output,
    cache_read: a.cache_read + b.cache_read,
    cache_write: a.cache_write + b.cache_write,
    reasoning: a.reasoning + b.reasoning,
    requests: a.requests + b.requests,
  }
}

function bucketTotal(b: UsageBucket): number {
  return b.input + b.output + b.cache_read + b.cache_write + b.reasoning
}

async function load() {
  loading.value = true
  try {
    let days: number | null = null
    let hours: number | null = null
    switch (rangeKey.value) {
      case 'h8': hours = 8; break
      case 'h12': hours = 12; break
      case 'h24': hours = 24; break
      case 'd7': days = 7; break
      case 'd90': days = 90; break
      case 'custom':
        days = Math.max(1, Math.min(3650, Math.floor(customDays.value || 1)))
        break
      // 'all'：全部时间
    }
    result.value = await invoke<TokenStatsResult>('query_token_stats', { days, hours })
    // 剔除当前范围已不存在的选中项，避免过滤出全空数据
    const keys = new Set(result.value.by_model.map(modelKey))
    selectedModels.value = selectedModels.value.filter(k => keys.has(k))
  } catch (e) {
    toast.showError(String(e))
  }
  loading.value = false
}

function modelKey(m: ModelStats): string {
  return `${m.source}|${m.provider}|${m.model}`
}

const modelOptions = computed(() =>
  (result.value?.by_model ?? []).map(m => ({
    key: modelKey(m),
    model: m.model,
    provider: m.provider,
    source: m.source,
  }))
)

function sourceMatch(s: string): boolean {
  return source.value === 'all' || s === source.value
}

/** 当前数据源 + 模型筛选下的每日 bucket */
function bucketOfDay(d: DayStats): UsageBucket {
  if (selectedModels.value.length > 0) {
    return d.models
      .filter(m => sourceMatch(m.source) && selectedModels.value.includes(modelKey(m)))
      .reduce((acc, m) => addBuckets(acc, m.usage), emptyBucket())
  }
  if (source.value === 'zcode') return d.zcode
  if (source.value === 'claude') return d.claude
  return addBuckets(d.zcode, d.claude)
}

const totals = computed<UsageBucket>(() => {
  if (!result.value) return emptyBucket()
  const r = result.value
  if (selectedModels.value.length > 0) {
    return r.by_model
      .filter(m => sourceMatch(m.source) && selectedModels.value.includes(modelKey(m)))
      .reduce((acc, m) => addBuckets(acc, m.usage), emptyBucket())
  }
  if (source.value === 'zcode') return r.totals_zcode
  if (source.value === 'claude') return r.totals_claude
  return addBuckets(r.totals_zcode, r.totals_claude)
})

const sessions = computed(() => {
  if (!result.value) return 0
  const r = result.value
  if (source.value === 'zcode') return r.zcode_sessions
  if (source.value === 'claude') return r.claude_sessions
  return r.zcode_sessions + r.claude_sessions
})

const summaryTiles = computed(() => [
  { label: '总 Token', value: fmt(bucketTotal(totals.value)), primary: true },
  { label: 'Input', value: fmt(totals.value.input) },
  { label: 'Output', value: fmt(totals.value.output) },
  { label: '缓存读取', value: fmt(totals.value.cache_read) },
  { label: '缓存写入', value: fmt(totals.value.cache_write) },
  { label: 'Reasoning', value: fmt(totals.value.reasoning) },
  { label: '请求数', value: fmt(totals.value.requests) },
  { label: '会话数', value: String(sessions.value) },
])

const chartDays = computed(() => {
  if (!result.value) return [] as { key: string; label: string; bucket: UsageBucket }[]
  const hourly = result.value.granularity === 'hour'
  return result.value.by_day.map(d => ({
    key: d.date,
    // 小时粒度 "YYYY-MM-DD HH:00" → "HH:00"，天粒度 "YYYY-MM-DD" → "MM-DD"
    label: hourly ? d.date.slice(11) : d.date.slice(5),
    bucket: bucketOfDay(d),
  }))
})

const hasData = computed(() => chartDays.value.length > 0)

const chartOption = computed(() => {
  const data = chartDays.value
  if (data.length === 0) return {}

  // 只展示当前范围内有数据的分段，避免空系列占据图例
  const visible = CATEGORIES.filter(c => data.some(d => d.bucket[c.key] > 0))
  const series = visible.map(c => ({
    name: c.label,
    type: 'bar',
    stack: 'total',
    data: data.map(d => d.bucket[c.key]),
    itemStyle: { color: c.color, opacity: 0.85 },
    barMaxWidth: 18,
  }))
  // 堆叠最上层（最后渲染）的系列顶部圆角
  if (series.length > 0) {
    ;(series[series.length - 1] as { itemStyle: Record<string, unknown> }).itemStyle.borderRadius = [3, 3, 0, 0]
  }

  // 数据档位多时压缩底部留白给滚动条
  const crowded = data.length > 30
  const option: Record<string, unknown> = {
    animationDuration: 300,
    grid: { left: 8, right: 8, top: 32, bottom: crowded ? 24 : 4, containLabel: true },
    legend: {
      top: 0,
      left: 0,
      itemWidth: 10,
      itemHeight: 10,
      itemGap: 14,
      icon: 'roundRect',
      textStyle: { fontSize: 11, color: AXIS_COLOR },
    },
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'shadow', shadowStyle: { color: 'rgba(128, 128, 128, 0.08)' } },
      confine: true,
      textStyle: { fontSize: 11 },
      formatter: (params: { dataIndex: number; marker: string; name: string; seriesName: string; value: number }[]) => {
        const first = params[0]
        const title = result.value?.granularity === 'hour'
          ? (chartDays.value[first?.dataIndex ?? 0]?.key ?? first?.name ?? '')
          : `${new Date().getFullYear()}-${first?.name ?? ''}`
        const lines = [title]
        let sum = 0
        for (const p of params) {
          sum += p.value ?? 0
          lines.push(`${p.marker}${p.seriesName}：${fmt(p.value ?? 0)}`)
        }
        lines.push(`总计：${fmt(sum)}`)
        return lines.join('<br/>')
      },
    },
    xAxis: {
      type: 'category',
      data: data.map(d => d.label),
      axisTick: { show: false },
      axisLine: { lineStyle: { color: 'rgba(128, 128, 128, 0.3)' } },
      axisLabel: { color: AXIS_COLOR, fontSize: 10 },
    },
    yAxis: {
      type: 'value',
      axisLabel: { color: AXIS_COLOR, fontSize: 10, formatter: (v: number) => fmt(v) },
      splitLine: { lineStyle: { color: 'rgba(128, 128, 128, 0.15)' } },
    },
    series,
  }

  // 数据档位多时启用滚轮缩放 + 底部滚动条，默认展示最近 30 档
  if (crowded) {
    option.dataZoom = [
      { type: 'inside', zoomOnMouseWheel: true, moveOnMouseWheel: true },
      {
        type: 'slider',
        height: 12,
        bottom: 2,
        showDetail: false,
        startValue: Math.max(0, data.length - 30),
        endValue: data.length - 1,
        borderColor: 'transparent',
        backgroundColor: 'rgba(128, 128, 128, 0.1)',
        fillerColor: 'rgba(56, 89, 255, 0.15)',
        moveHandleSize: 0,
        handleStyle: { color: '#fff', borderColor: '#3859ff' },
      },
    ]
  }
  return option
})

const filteredModels = computed(() => {
  if (!result.value) return []
  return result.value.by_model.filter(
    m => sourceMatch(m.source)
      && (selectedModels.value.length === 0 || selectedModels.value.includes(modelKey(m)))
  )
})

function fmt(n: number): string {
  if (n >= 1_000_000_000) return (n / 1_000_000_000).toFixed(2) + 'B'
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + 'M'
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'K'
  return String(n)
}

function sourceLabel(s: string): string {
  return s === 'zcode' ? 'Zcode' : 'Claude Code'
}

function onDocClick(e: MouseEvent) {
  if (modelMenuOpen.value && modelMenuRef.value && !modelMenuRef.value.contains(e.target as Node)) {
    modelMenuOpen.value = false
  }
}

onMounted(() => {
  load()
  document.addEventListener('click', onDocClick)
})

onUnmounted(() => document.removeEventListener('click', onDocClick))
</script>

<template>
  <div class="token-stats">
    <h2 class="page-title">Token 统计</h2>

    <SettingCard title="用量统计" description="基于 Zcode 与 Claude Code 本地会话日志聚合" icon-variant="accent">
      <template #icon>
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="6" y1="20" x2="6" y2="16"/>
          <line x1="12" y1="20" x2="12" y2="10"/>
          <line x1="18" y1="20" x2="18" y2="4"/>
        </svg>
      </template>
      <template #action>
        <button class="dev-btn" :disabled="loading" @click="load">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18.52 13.45a8 8 0 0 1-11.06 5.56"/><path d="M5.48 10.55a8 8 0 0 1 11.06-5.56"/>
            <polyline points="15 2 18.54 5.46 15.01 8.99"/><polyline points="9 22 5.46 18.54 8.99 15.01"/>
          </svg>
          {{ loading ? '统计中...' : '刷新' }}
        </button>
      </template>

      <!-- 筛选 -->
      <div class="filters">
        <div class="seg-group">
          <button
            v-for="opt in sourceOptions"
            :key="opt.value"
            :class="['seg-btn', { active: source === opt.value }]"
            @click="source = opt.value"
          >
            {{ opt.label }}
          </button>
        </div>
        <select v-model="rangeKey" class="days-select" @change="load">
          <option v-for="opt in rangeOptions" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
        </select>
        <div v-if="rangeKey === 'custom'" class="custom-range">
          <input
            v-model.number="customDays"
            type="number"
            min="1"
            max="3650"
            class="custom-input"
            @change="load"
            @keyup.enter="load"
          />
          <span class="custom-unit">天</span>
        </div>

        <!-- 模型多选筛选 -->
        <div v-if="modelOptions.length" ref="modelMenuRef" class="model-filter">
          <button class="days-select model-toggle" @click.stop="toggleModelMenu">
            模型{{ selectedModels.length ? ` · ${selectedModels.length}` : '' }}
            <svg
              width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor"
              stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"
              class="chev" :class="{ open: modelMenuOpen }"
            ><polyline points="6 9 12 15 18 9"/></svg>
          </button>
          <div v-if="modelMenuOpen" class="model-menu" :style="menuStyle" @click.stop>
            <div class="model-menu-head">
              <span class="model-menu-title">已选 {{ selectedModels.length }}/{{ modelOptions.length }}</span>
              <button class="model-clear" :disabled="!selectedModels.length" @click="selectedModels = []">清除</button>
            </div>
            <div class="model-list">
              <label v-for="opt in modelOptions" :key="opt.key" class="model-option">
                <input v-model="selectedModels" type="checkbox" :value="opt.key" class="model-check" />
                <span class="model-name">{{ opt.model }}</span>
                <span :class="['source-tag', opt.source]">{{ sourceLabel(opt.source) }}</span>
                <span v-if="opt.provider" class="model-provider" :title="opt.provider">{{ opt.provider }}</span>
              </label>
            </div>
          </div>
        </div>
      </div>

      <!-- 内部 loading：首次加载 / 无数据时扫描中 -->
      <div v-if="loading && !hasData" class="loading-block">
        <span class="spinner" />
        <span>正在扫描本地会话日志...</span>
      </div>

      <template v-else>
        <!-- 数据源缺失提示 -->
        <div v-if="result && !result.zcode_detected" class="source-hint">未找到 Zcode 日志目录 (~/.zcode/cli/rollout)</div>
        <div v-if="result && !result.claude_detected" class="source-hint">未找到 Claude Code 日志目录 (~/.claude/projects)</div>
        <div v-if="!hasData" class="source-hint">所选范围内没有用量数据</div>

        <template v-if="hasData">
          <!-- 汇总磁贴 -->
          <div class="summary-grid">
            <div
              v-for="tile in summaryTiles"
              :key="tile.label"
              :class="['stat-tile', { primary: tile.primary }]"
            >
              <span class="stat-label">{{ tile.label }}</span>
              <span class="stat-value">{{ tile.value }}</span>
            </div>
          </div>

          <!-- 按时用量（分类堆叠柱状图） -->
          <div class="chart-panel">
            <v-chart :option="chartOption" class="chart-canvas" autoresize />
          </div>
        </template>
      </template>

      <!-- 按模型表格 -->
      <div v-if="filteredModels.length > 0" class="table-card">
        <table class="model-table">
          <thead>
            <tr>
              <th>模型</th>
              <th>来源</th>
              <th class="num">请求数</th>
              <th class="num">Input</th>
              <th class="num">Output</th>
              <th class="num">缓存</th>
              <th class="num">合计</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="m in filteredModels" :key="m.source + m.provider + m.model">
              <td class="model-name">
                {{ m.model }}
                <span v-if="m.provider" class="provider-tag" :title="m.provider">{{ m.provider }}</span>
              </td>
              <td>
                <span :class="['source-tag', m.source]">{{ sourceLabel(m.source) }}</span>
              </td>
              <td class="num">{{ fmt(m.usage.requests) }}</td>
              <td class="num">{{ fmt(m.usage.input) }}</td>
              <td class="num">{{ fmt(m.usage.output) }}</td>
              <td class="num">{{ fmt(m.usage.cache_read + m.usage.cache_write) }}</td>
              <td class="num strong">{{ fmt(bucketTotal(m.usage)) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </SettingCard>
  </div>
</template>

<style scoped>
.token-stats {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.page-title {
  font-size: 22px;
  font-weight: 700;
  color: var(--text);
  margin-bottom: 8px;
}

.filters {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 16px;
  flex-wrap: wrap;
}

.seg-group {
  display: inline-flex;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  padding: 2px;
  gap: 2px;
}

.seg-btn {
  padding: 5px 14px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s;
  white-space: nowrap;
}

.seg-btn:hover {
  color: var(--text);
}

.seg-btn.active {
  background: var(--accent);
  color: #fff;
}

.days-select {
  padding: 6px 10px;
  background: var(--bg);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  font-size: 12px;
  cursor: pointer;
}

.custom-range {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.custom-input {
  width: 64px;
  padding: 5px 8px;
  background: var(--bg);
  color: var(--text);
  border: 1px solid var(--accent);
  border-radius: var(--radius-xs);
  font-size: 12px;
  text-align: center;
}

.custom-unit {
  font-size: 12px;
  color: var(--text-secondary);
}

/* ---- 模型多选筛选 ---- */
.model-filter {
  position: relative;
}

.model-toggle {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}

.chev {
  transition: transform 0.15s;
}

.chev.open {
  transform: rotate(180deg);
}

.model-menu {
  position: absolute;
  z-index: 40;
  width: 320px;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg);
  padding: 8px;
}

.model-menu-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 2px 6px 8px;
  border-bottom: 1px solid var(--border);
  margin-bottom: 6px;
}

.model-menu-title {
  font-size: 11px;
  color: var(--text-secondary);
}

.model-clear {
  border: none;
  background: none;
  padding: 0;
  color: var(--accent);
  font-size: 11px;
  cursor: pointer;
}

.model-clear:disabled {
  color: var(--text-secondary);
  opacity: 0.5;
  cursor: default;
}

.model-list {
  max-height: 240px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.model-option {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px;
  border-radius: var(--radius-xs);
  cursor: pointer;
  font-size: 12px;
  color: var(--text);
}

.model-option:hover {
  background: var(--bg);
}

.model-check {
  width: 14px;
  height: 14px;
  margin: 0;
  flex-shrink: 0;
  accent-color: var(--accent);
  cursor: pointer;
}

.model-provider {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 10px;
  color: var(--text-secondary);
  text-align: right;
}

.loading-block {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  padding: 56px 0;
  color: var(--text-secondary);
  font-size: 13px;
}

.spinner {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  border: 3px solid var(--border);
  border-top-color: var(--accent);
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.source-hint {
  font-size: 12px;
  color: var(--text-secondary);
  background: var(--bg);
  padding: 8px 12px;
  border-radius: var(--radius-xs);
  margin-bottom: 12px;
}

/* ---- 汇总磁贴 ---- */
.summary-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
  margin-bottom: 16px;
}

.stat-tile {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 10px 12px;
  background: var(--bg);
  border-radius: var(--radius-xs);
  min-width: 0;
}

.stat-tile.primary {
  background: var(--accent-light);
  box-shadow: inset 0 0 0 1px var(--accent);
}

.stat-label {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
}

.stat-value {
  font-size: 16px;
  font-weight: 600;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.stat-tile.primary .stat-value {
  color: var(--accent);
  font-weight: 700;
}

/* ---- 柱状图 ---- */
.chart-panel {
  margin-bottom: 18px;
}

.chart-canvas {
  height: 280px;
  width: 100%;
}

/* ---- 模型表格 ---- */
.table-card {
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  /* 列不换行导致超宽时显示横向滚动条而不是裁切 */
  overflow-x: auto;
}

/* 细滚动条（表格横向 + 模型下拉纵向） */
.table-card::-webkit-scrollbar,
.model-list::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}

.table-card::-webkit-scrollbar-track,
.model-list::-webkit-scrollbar-track {
  background: transparent;
}

.table-card::-webkit-scrollbar-thumb,
.model-list::-webkit-scrollbar-thumb {
  background: rgba(128, 128, 128, 0.35);
  border-radius: 3px;
}

.table-card::-webkit-scrollbar-thumb:hover,
.model-list::-webkit-scrollbar-thumb:hover {
  background: rgba(128, 128, 128, 0.55);
}

.model-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.model-table th {
  text-align: left;
  padding: 9px 12px;
  color: var(--text-secondary);
  font-weight: 500;
  background: var(--bg);
  white-space: nowrap;
}

.model-table td {
  padding: 9px 12px;
  color: var(--text);
  white-space: nowrap;
}

.model-table tbody tr {
  border-top: 1px solid var(--border);
  transition: background 0.12s;
}

.model-table tbody tr:hover {
  background: var(--accent-light);
}

.model-table .num {
  text-align: right;
}

.model-table .strong {
  font-weight: 600;
  color: var(--accent);
}

.model-name {
  font-family: ui-monospace, SFMono-Regular, monospace;
}

.provider-tag {
  display: inline-block;
  margin-left: 8px;
  padding: 1px 8px;
  border-radius: 9px;
  font-size: 10px;
  font-weight: 500;
  font-family: inherit;
  background: var(--bg);
  color: var(--text-secondary);
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  vertical-align: middle;
}

.source-tag {
  display: inline-block;
  padding: 1px 8px;
  border-radius: 9px;
  font-size: 10px;
  font-weight: 500;
  white-space: nowrap;
}

.source-tag.zcode {
  background: rgba(139, 92, 246, 0.1);
  color: #8b5cf6;
}

.source-tag.claude {
  background: var(--accent-light);
  color: var(--accent);
}

.dev-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  color: var(--text);
  font-size: 12px;
  font-weight: 500;
  transition: all 0.15s;
  cursor: pointer;
}

.dev-btn:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent);
}

.dev-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
