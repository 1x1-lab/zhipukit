import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useToast } from './useToast'

export interface BalanceInfo {
  balance: number
  recharge_amount: number
  give_amount: number
  total_spend_amount: number
  frozen_balance: number
  available_balance: number
}

export interface CodingPlanInfo {
  level: string
  /** 套餐版本：v2 / v3；未知或旧缓存为空 */
  plan_version?: string
  hour5_percentage: number
  hour5_next_reset: number
  weekly_percentage: number
  weekly_next_reset: number
  mcp_total: number
  mcp_used: number
  mcp_remaining: number
  mcp_next_reset: number
  /** V3（积分制）5 小时窗口的积分总量 / 已用 / 剩余，V2 为 0（旧缓存可能缺失该字段） */
  hour5_credits_total?: number
  hour5_credits_used?: number
  hour5_credits_remaining?: number
  /** V3（积分制）每周窗口的积分总量 / 已用 / 剩余，V2 为 0 */
  weekly_credits_total?: number
  weekly_credits_used?: number
  weekly_credits_remaining?: number
}

interface CacheEntry<T> {
  timestamp: number
  data: T
}

function getCacheTTL(): number {
  const val = Number(localStorage.getItem('zhipu_cache_duration'))
  return val > 0 ? val * 1000 : 60_000
}
const KEY_BALANCE = 'zhipu_cache_balance'
const KEY_PLAN = 'zhipu_cache_plan'
const KEY_LAST_REFRESH = 'zhipu_last_refresh'

const balance = ref<BalanceInfo | null>(null)
const codingPlan = ref<CodingPlanInfo | null>(null)
const loading = ref(false)

const toast = useToast()

function writeCache(data: { balance?: BalanceInfo; plan?: CodingPlanInfo }) {
  const now = Date.now()
  if (data.balance) {
    localStorage.setItem(KEY_BALANCE, JSON.stringify({ timestamp: now, data: data.balance }))
    balance.value = data.balance
  }
  if (data.plan) {
    localStorage.setItem(KEY_PLAN, JSON.stringify({ timestamp: now, data: data.plan }))
    codingPlan.value = data.plan
  }
  localStorage.setItem(KEY_LAST_REFRESH, String(now))
}

function syncTrayData() {
  invoke('update_tray_data', {
    balance: balance.value,
    codingPlan: codingPlan.value,
  }).catch(() => {})
}

/** 真正查询（调 Rust invoke），结果写入缓存 */
export async function fetchFresh(apiKey: string, endpoint: string) {
  if (!apiKey) {
    toast.showWarning('请先在设置中配置 API Key')
    return
  }
  if (loading.value) return

  loading.value = true

  const errors: string[] = []
  const results = await Promise.allSettled([
    invoke<BalanceInfo>('query_balance', { apiKey, endpoint }),
    invoke<CodingPlanInfo>('query_coding_plan', { apiKey, endpoint }),
  ])

  const bResult = results[0]
  const pResult = results[1]

  let newBalance: BalanceInfo | undefined
  let newPlan: CodingPlanInfo | undefined

  if (bResult.status === 'fulfilled') {
    newBalance = bResult.value
  } else {
    errors.push(`${bResult.reason}`)
  }

  if (pResult.status === 'fulfilled') {
    newPlan = pResult.value
  } else {
    errors.push(`${pResult.reason}`)
  }

  if (newBalance || newPlan) {
    writeCache({ balance: newBalance, plan: newPlan })
    syncTrayData()
  } else {
    toast.showError(errors.join('；'))
  }

  loading.value = false
}

/** 读缓存：<= 1 分钟直接返回，否则调 fetchFresh */
export async function fetchCached(apiKey: string, endpoint: string) {
  try {
    const rawBalance = localStorage.getItem(KEY_BALANCE)
    const rawPlan = localStorage.getItem(KEY_PLAN)
    const now = Date.now()

    if (rawBalance && rawPlan) {
      const cachedBalance: CacheEntry<BalanceInfo> = JSON.parse(rawBalance)
      const cachedPlan: CacheEntry<CodingPlanInfo> = JSON.parse(rawPlan)

      const ttl = getCacheTTL()
      if (now - cachedBalance.timestamp < ttl && now - cachedPlan.timestamp < ttl) {
        balance.value = cachedBalance.data
        codingPlan.value = cachedPlan.data
        return
      }
    }
  } catch {
    // cache corrupt, fall through to fetchFresh
  }

  await fetchFresh(apiKey, endpoint)
}

/** 同步恢复缓存数据到 refs（不检查 TTL，用于组件挂载时立即显示） */
export function restoreFromCache() {
  try {
    const rawBalance = localStorage.getItem(KEY_BALANCE)
    const rawPlan = localStorage.getItem(KEY_PLAN)

    if (rawBalance) {
      balance.value = (JSON.parse(rawBalance) as CacheEntry<BalanceInfo>).data
    }
    if (rawPlan) {
      codingPlan.value = (JSON.parse(rawPlan) as CacheEntry<CodingPlanInfo>).data
    }
  } catch {
    // ignore
  }
}

/** 注册 Rust 后端事件监听，返回 unsubscribe 函数 */
export async function setupListeners(): Promise<() => void> {
  const unlistenBalance = await listen<Record<string, unknown>>('balance-update', (e) => {
    const d = e.payload
    const b: BalanceInfo = {
      balance: (d.balance as number) ?? 0,
      recharge_amount: (d.rechargeAmount as number) ?? 0,
      give_amount: (d.giveAmount as number) ?? 0,
      total_spend_amount: (d.totalSpendAmount as number) ?? 0,
      frozen_balance: (d.frozenBalance as number) ?? 0,
      available_balance: (d.availableBalance as number) ?? 0,
    }
    writeCache({ balance: b })
    syncTrayData()
  })

  // 自动刷新由 Rust 端用 query_coding_plan 同一份逻辑解析，载荷即 CodingPlanInfo
  const unlistenPlan = await listen<CodingPlanInfo>('plan-update', (e) => {
    writeCache({ plan: e.payload })
    syncTrayData()
  })

  return () => {
    unlistenBalance()
    unlistenPlan()
  }
}

export function useBalanceCache() {
  return {
    balance,
    codingPlan,
    loading,
    fetchFresh,
    fetchCached,
    restoreFromCache,
    setupListeners,
  }
}
