import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'path'

// https://vite.dev/config/
export default defineConfig({
  plugins: [vue()],
  server: {
    host: '127.0.0.1',
    // 1420 是 Tauri 惯用端口，避开 vite 默认端口段（5173/5174）与其他项目冲突
    port: 1420,
    strictPort: true,
    watch: {
      // 忽略 Rust 构建产物，避免监听正在写入的 exe 触发 EBUSY
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        'tray-popup': resolve(__dirname, 'tray-popup.html'),
      },
    },
  },
})
