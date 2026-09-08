<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useToast } from '../composables/useToast'
import SettingCard from '../components/SettingCard.vue'
import ToggleBtn from '../components/ToggleBtn.vue'

const toast = useToast()

const props = defineProps<{
  zhipuApiKey: string
}>()

const localApiKey = ref(localStorage.getItem('zhipu_api_key') || '')

interface ZCodeStatus {
  installed: boolean
  version: string | null
  path: string | null
  config_path: string | null
}

interface ZCodeProvider {
  id: string
  name: string
  kind: string
  source: string
  api_key: string
  base_url: string
}

interface ZCodeConfig {
  config_path: string
  providers: ZCodeProvider[]
}

const status = ref<ZCodeStatus | null>(null)
const configPath = ref('')
const providers = ref<ZCodeProvider[]>([])
const showKeys = ref<Record<string, boolean>>({})
const loading = ref(false)
const saved = ref(false)

const copiedPath = ref('')

async function detect() {
  try {
    status.value = await invoke<ZCodeStatus>('detect_zcode')
  } catch (e) {
    toast.showError(String(e))
  }
}

async function loadConfig() {
  try {
    const config = await invoke<ZCodeConfig>('read_zcode_config')
    configPath.value = config.config_path
    providers.value = config.providers
    // ToggleBtn 要求 boolean，预填每个 provider 的显隐状态避免 undefined
    showKeys.value = Object.fromEntries(config.providers.map(p => [p.id, false]))
  } catch (e) {
    toast.showError(String(e))
  }
}

async function refresh() {
  loading.value = true
  await detect()
  if (status.value?.installed) {
    await loadConfig()
  }
  loading.value = false
}

async function saveConfig() {
  try {
    await invoke('save_zcode_config', {
      providers: providers.value.map(p => ({
        id: p.id,
        api_key: p.api_key,
        base_url: p.base_url,
      })),
    })
    saved.value = true
    setTimeout(() => { saved.value = false }, 2000)
  } catch (e) {
    toast.showError(String(e))
  }
}

function fillKey(provider: ZCodeProvider) {
  const key = props.zhipuApiKey || localApiKey.value
  if (key) {
    provider.api_key = key
  }
}

function maskKey(key: string): string {
  if (!key) return ''
  if (key.length <= 8) return '****'
  return key.slice(0, 4) + '****' + key.slice(-4)
}

function copyToClipboard(text: string) {
  navigator.clipboard.writeText(text)
  copiedPath.value = text
  setTimeout(() => { copiedPath.value = '' }, 1500)
}

onMounted(() => refresh())
</script>

<template>
  <div class="zcode-config">
    <h2 class="page-title">Zcode 配置</h2>

    <!-- 安装状态 -->
    <SettingCard title="安装状态" icon-variant="purple">
      <template #icon>
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="16 18 22 12 16 6"/>
          <polyline points="8 6 2 12 8 18"/>
        </svg>
      </template>
      <template #description>
        <span v-if="loading">检测中...</span>
        <span v-else-if="status?.installed" class="status-installed">已安装</span>
        <span v-else class="status-not-found">未检测到 Zcode</span>
      </template>
      <template #action>
        <button class="dev-btn" @click="refresh">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18.52 13.45a8 8 0 0 1-11.06 5.56"/><path d="M5.48 10.55a8 8 0 0 1 11.06-5.56"/>
            <polyline points="15 2 18.54 5.46 15.01 8.99"/><polyline points="9 22 5.46 18.54 8.99 15.01"/>
          </svg>
          重新检测
        </button>
      </template>
      <div v-if="status?.path" class="path-display">
        <span class="path-label">安装位置</span>
        <code class="path-value">{{ status.path }}</code>
        <button class="copy-btn" @click="copyToClipboard(status.path!)" :title="copiedPath === status.path ? '已复制' : '复制'">
          <svg v-if="copiedPath !== status.path" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
          <svg v-else width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
        </button>
      </div>
      <div v-if="status?.config_path" class="path-display" :style="{ marginTop: status?.path ? '4px' : undefined }">
        <span class="path-label">配置文件</span>
        <code class="path-value">{{ status.config_path }}</code>
        <button class="copy-btn" @click="copyToClipboard(status.config_path!)" :title="copiedPath === status.config_path ? '已复制' : '复制'">
          <svg v-if="copiedPath !== status.config_path" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
          <svg v-else width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
        </button>
      </div>
    </SettingCard>

    <!-- Provider 配置（仅安装后显示） -->
    <template v-if="status?.installed && providers.length > 0">
      <SettingCard
        v-for="provider in providers"
        :key="provider.id"
        :title="provider.name || provider.id"
        :description="provider.id"
        icon-variant="accent"
      >
        <template #icon>
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="5" width="20" height="14" rx="2"/>
            <path d="M2 10h20"/>
          </svg>
        </template>
        <template #action>
          <span :class="['key-badge', provider.api_key ? 'configured' : 'empty']">
            {{ provider.api_key ? '已配置' : '未配置' }}
          </span>
          <button
            v-if="provider.id === 'builtin:bigmodel'"
            class="fill-btn"
            :disabled="!zhipuApiKey && !localApiKey"
            @click="fillKey(provider)"
            title="将设置中的 API Key 填入此字段"
          >
            一键填入
          </button>
        </template>
        <div class="fields">
          <div class="field-row">
            <label class="field-label">API Key</label>
            <div class="input-group">
              <input
                :type="showKeys[provider.id] ? 'text' : 'password'"
                v-model="provider.api_key"
                class="input-field"
                placeholder="输入 API Key"
              />
              <ToggleBtn v-model="showKeys[provider.id]" />
            </div>
          </div>
          <div class="field-row">
            <label class="field-label">Base URL</label>
            <input v-model="provider.base_url" class="input-field" />
          </div>
          <div v-if="provider.api_key" class="key-preview">{{ maskKey(provider.api_key) }}</div>
        </div>
      </SettingCard>

      <button class="save-btn" @click="saveConfig">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="20 6 9 17 4 12"/>
        </svg>
        {{ saved ? '已保存' : '保存配置' }}
      </button>
    </template>
  </div>
</template>

<style scoped>
.zcode-config {
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

.status-installed {
  color: var(--success);
  font-weight: 500;
}

.status-not-found {
  color: var(--text-secondary);
}

.path-display {
  display: flex;
  align-items: center;
  gap: 8px;
}

.path-label {
  font-size: 11px;
  color: var(--text-secondary);
  flex-shrink: 0;
}

.path-value {
  font-size: 11px;
  font-family: ui-monospace, SFMono-Regular, monospace;
  color: var(--text-secondary);
  background: var(--bg);
  padding: 2px 8px;
  border-radius: 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.copy-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  background: none;
  border: 1px solid transparent;
  border-radius: 4px;
  color: var(--text-secondary);
  cursor: pointer;
  flex-shrink: 0;
  transition: all 0.15s;
}

.copy-btn:hover {
  color: var(--accent);
  border-color: var(--accent);
  background: var(--accent-light);
}

.fields {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.field-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.field-label {
  font-size: 11px;
  color: var(--text-secondary);
  width: 180px;
  flex-shrink: 0;
  font-family: ui-monospace, SFMono-Regular, monospace;
}

.input-group {
  display: flex;
  gap: 8px;
  flex: 1;
  min-width: 0;
}

.input-field {
  flex: 1;
  min-width: 0;
}

.key-preview {
  margin-top: 4px;
  font-size: 12px;
  color: var(--text-secondary);
  font-family: ui-monospace, monospace;
}

.key-badge {
  display: inline-flex;
  align-items: center;
  padding: 3px 10px;
  border-radius: 10px;
  font-size: 11px;
  font-weight: 500;
  white-space: nowrap;
}

.key-badge.configured {
  background: var(--success-light);
  color: var(--success);
}

.key-badge.empty {
  background: var(--warning-light);
  color: var(--warning);
}

.fill-btn {
  margin-left: 8px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 5px 12px;
  background: var(--accent-light);
  border: 1px solid var(--accent);
  border-radius: var(--radius-xs);
  color: var(--accent);
  font-size: 11px;
  font-weight: 600;
  transition: all 0.15s;
  white-space: nowrap;
}

.fill-btn:hover:not(:disabled) {
  background: var(--accent);
  color: #fff;
}

.fill-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.save-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 6px 16px;
  background: var(--accent-gradient);
  color: #fff;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 600;
  transition: opacity 0.2s;
  margin-left: 48px;
  border: none;
  cursor: pointer;
}

.save-btn:hover { opacity: 0.9; }

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

.dev-btn:hover {
  border-color: var(--accent);
  color: var(--accent);
}
</style>
