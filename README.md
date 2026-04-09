# 智谱 Token 查询工具

一款基于 Tauri 2 + Vue 3 构建的轻量桌面工具，用于查询智谱 AI（Zhipu / Z.ai）账户余额、Coding Plan 配额和 Token 用量计算。

## 功能

### 余额查询
- 账户余额（当前余额、可用余额、冻结余额）
- 累计充值、赠送金额、累计消费
- Coding Plan 配额
  - 5 小时 Token 限额（使用百分比 + 重置倒计时）
  - 每周边额（部分套餐生效）
  - MCP 月度调用次数（已用/总量）
- 支持自动刷新（10 秒 ~ 5 分钟可调）

### Token 计算
- 支持多模型选择（GLM-4-Plus / GLM-4 / GLM-4-Air / GLM-4-Long / GLM-4-Flash / GLM-4.5 等）
- 本地快速估算 + API 精确计算
- 预估费用计算

### 设置
- API Key 管理（本地存储）
- 支持国内版（open.bigmodel.cn）和国际版（api.z.ai）
- 调试工具（API 连通性测试）
- 开发者工具（DevTools、应用信息）

## 技术栈

| 层 | 技术 |
|---|---|
| 前端 | Vue 3 + TypeScript + Vite |
| 后端 | Rust + Tauri 2 |
| HTTP | reqwest |
| 异步 | tokio |

## 开发

```bash
# 安装依赖
npm install

# 开发模式
npm run tauri dev

# 构建生产包
npm run tauri build
```

### 前置要求

- Node.js >= 18
- Rust >= 1.77
- Tauri CLI 2.x

## 项目结构

```
src/                      # 前端源码
  App.vue                 # 主布局（侧边栏 + 页面切换）
  main.ts                 # 入口
  components/
    BalanceQuery.vue      # 余额查询页面
    TokenCalculator.vue   # Token 计算页面
    SettingsView.vue      # 设置页面
src-tauri/
  src/
    lib.rs                # Rust 后端（API 调用、自动刷新、Token 计算）
  tauri.conf.json         # Tauri 配置
  Cargo.toml              # Rust 依赖
```

## API 端点

| 功能 | 端点 |
|---|---|
| 余额查询 | `GET /api/biz/account/query-customer-account-report` |
| Coding Plan 配额 | `GET /api/monitor/usage/quota/limit` |
| Token 计算 | `POST /api/paas/v4/chat/completions` |

## License

MIT
