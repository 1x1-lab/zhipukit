use std::sync::Mutex;
use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

mod api;
mod claude;
mod tray;
mod usage;
mod zcode;
pub mod types;
pub mod utils;

use types::AppState;
use tray::show_popup;

#[cfg(target_os = "macos")]
use tray::set_tray_highlight;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .manage(AppState {
            client: reqwest::Client::new(),
            refresh_handle: Mutex::new(None),
            tray_data: Mutex::new(types::TrayData::default()),
            minimize_to_tray: Mutex::new(false),
            main_hidden_at: Mutex::new(None),
        })
        .setup(|app| {
            // 主窗口默认不可见，仅在非自启时显示
            let is_autostart = std::env::args().any(|a| a == "--autostart");
            if is_autostart {
                // 自启：保持隐藏，直接托盘运行
                let state = app.state::<AppState>();
                *state.minimize_to_tray.lock().unwrap() = true;
                // 自启时主窗口从启动开始就是隐藏状态，记录起点以便首次打开时
                // 使用已有的隐藏窗口重载策略。
                *state.main_hidden_at.lock().unwrap() = Some(std::time::Instant::now());
                #[cfg(target_os = "macos")]
                let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            } else {
                // 手动启动：显示主窗口
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                // 读取持久化的关闭行为设置
                let persisted = tray::read_minimize_setting().unwrap_or(false);
                let state = app.state::<AppState>();
                *state.minimize_to_tray.lock().unwrap() = persisted;
            }

            // macOS 的 DoubleClick 事件不受支持；保留原生右键菜单作为
            // popup WebView 异常时的恢复入口。
            let tray_menu = MenuBuilder::new(app)
                .text("tray-show-main", "显示窗口")
                .separator()
                .text("tray-exit", "退出")
                .build()?;

            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .icon_as_template(true)
                .tooltip("ZhipuKit")
                .menu(&tray_menu)
                // 左键继续打开自定义 popup，右键显示原生菜单。
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "tray-show-main" => {
                        let state = app.state::<AppState>();
                        if let Err(error) = tray::show_main_window(app, &state) {
                            log::error!("原生托盘菜单显示主窗口失败: {}", error);
                        }
                    }
                    "tray-exit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state,
                            ..
                        } => {
                            match button_state {
                                MouseButtonState::Down => {
                                    let app = tray.app_handle();
                                    if let Err(error) = show_popup(&app) {
                                        log::error!("显示托盘 popup 失败: {}", error);
                                    }
                                }
                                MouseButtonState::Up => {
                                    #[cfg(target_os = "macos")]
                                    set_tray_highlight(&tray.app_handle(), false);
                                }
                            }
                        }
                        TrayIconEvent::DoubleClick { .. } => {
                            let app = tray.app_handle();
                            let state = app.state::<AppState>();
                            if let Err(error) = tray::show_main_window(&app, &state) {
                                log::error!("托盘双击显示主窗口失败: {}", error);
                            }
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // 监听主窗口关闭事件
            if let Some(window) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let state = app_handle.state::<AppState>();
                        let minimize = state.minimize_to_tray.lock().unwrap();

                        if !*minimize {
                            // 首次关闭：阻止关闭，让前端弹窗确认
                            api.prevent_close();
                            let _ = app_handle.emit("confirm-minimize-to-tray", ());
                        } else {
                            // 已确认过，直接隐藏到托盘
                            api.prevent_close();
                            if let Some(w) = app_handle.get_webview_window("main") {
                                let _ = w.hide();
                            }
                            // 记录隐藏时刻，用于判断是否需要重新加载
                            {
                                let state = app_handle.state::<AppState>();
                                *state.main_hidden_at.lock().unwrap() = Some(std::time::Instant::now());
                            }
                            #[cfg(target_os = "macos")]
                            let _ = app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                        }
                    }
                });
            }

            // popup 失焦或主窗口移动时自动隐藏
            if let Some(popup) = app.get_webview_window("tray-popup") {
                let app_handle = app.handle().clone();
                popup.on_window_event(move |event| {
                    match event {
                        tauri::WindowEvent::Focused(false) => {
                            if let Some(p) = app_handle.get_webview_window("tray-popup") {
                                let _ = p.hide();
                            }
                        }
                        _ => {}
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::query_balance,
            api::query_coding_plan,
            api::start_auto_refresh,
            api::stop_auto_refresh,
            tray::open_devtools,
            tray::get_app_info,
            tray::open_url,
            tray::update_tray_data,
            tray::confirm_minimize_to_tray,
            tray::get_minimize_to_tray,
            tray::set_minimize_to_tray,
            tray::exit_app,
            tray::start_window_drag,
            tray::get_tray_popup_data,
            tray::tray_show_main,
            tray::resize_popup,
            claude::detect_claude_code,
            claude::read_claude_config,
            claude::save_claude_config,
            claude::setup_claude_hook,
            claude::check_claude_hook_status,
            claude::test_zhipukit_status,
            claude::save_zhipu_endpoint,
            claude::save_zhipu_cache_duration,
            claude::read_segment_config,
            claude::save_segment_config,
            claude::read_bar_colors,
            claude::save_bar_colors,
            zcode::detect_zcode,
            zcode::read_zcode_config,
            zcode::save_zcode_config,
            usage::query_token_stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
