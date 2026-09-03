use std::{net::TcpListener, sync::Arc};

use gpui::{ClickEvent, ClipboardItem, FocusHandle, Subscription};
use ring::rand::{SecureRandom, SystemRandom};
use zed_ui::prelude::*;

use crate::{
    application::Application,
    mcp_runtime::{McpServicePhase, McpServiceStatus},
};

use super::{localization::text, shell::ShellSettings};

#[derive(Clone)]
struct McpCredentials {
    port: u16,
    auth_token: String,
}

pub(super) struct McpServiceItem {
    application: Arc<Application>,
    status: Option<McpServiceStatus>,
    credentials: Option<McpCredentials>,
    busy: bool,
    notice: Option<Result<String, String>>,
    request_generation: u64,
    focus_handle: FocusHandle,
    settings: gpui::Entity<ShellSettings>,
    _settings_observation: Subscription,
}

impl McpServiceItem {
    pub(super) fn new(
        application: Arc<Application>,
        settings: gpui::Entity<ShellSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let mut item = Self {
            application,
            status: None,
            credentials: None,
            busy: false,
            notice: None,
            request_generation: 0,
            focus_handle: cx.focus_handle(),
            settings,
            _settings_observation: settings_observation,
        };
        item.refresh(cx);
        item
    }

    pub(super) fn label(&self) -> String {
        "MCP Server".to_string()
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    fn refresh_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(runtime) = self.application.mcp().cloned() else {
            self.notice = Some(Err("MCP runtime is not configured".to_string()));
            cx.notify();
            return;
        };
        self.request_generation = self.request_generation.saturating_add(1);
        let generation = self.request_generation;
        let status = gpui_tokio::Tokio::spawn(cx, async move { runtime.status().await });
        cx.spawn(async move |item, cx| {
            let result = status.await.map_err(|error| error.to_string());
            item.update(cx, |item, cx| {
                if item.request_generation != generation {
                    return;
                }
                match result {
                    Ok(status) => item.status = Some(status),
                    Err(error) => item.notice = Some(Err(error)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let credentials = match generate_credentials() {
            Ok(credentials) => credentials,
            Err(error) => {
                self.notice = Some(Err(error));
                cx.notify();
                return;
            }
        };
        self.run_operation(McpOperation::Start(credentials), cx);
    }

    fn stop(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.run_operation(McpOperation::Stop, cx);
    }

    fn restart(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let credentials = match generate_credentials() {
            Ok(credentials) => credentials,
            Err(error) => {
                self.notice = Some(Err(error));
                cx.notify();
                return;
            }
        };
        self.run_operation(McpOperation::Restart(credentials), cx);
    }

    fn run_operation(&mut self, operation: McpOperation, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(runtime) = self.application.mcp().cloned() else {
            self.notice = Some(Err("MCP runtime is not configured".to_string()));
            cx.notify();
            return;
        };
        self.request_generation = self.request_generation.saturating_add(1);
        let generation = self.request_generation;
        self.busy = true;
        self.notice = None;
        cx.notify();
        let next_credentials = operation.credentials().cloned();
        let operation = gpui_tokio::Tokio::spawn(cx, async move {
            match operation {
                McpOperation::Start(credentials) => {
                    runtime
                        .start(credentials.port, credentials.auth_token)
                        .await
                }
                McpOperation::Stop => runtime.stop().await,
                McpOperation::Restart(credentials) => {
                    runtime
                        .restart(credentials.port, credentials.auth_token)
                        .await
                }
            }
        });
        cx.spawn(async move |item, cx| {
            let result = match operation.await {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
            item.update(cx, |item, cx| {
                if item.request_generation != generation {
                    return;
                }
                item.busy = false;
                match result {
                    Ok(status) => {
                        item.status = Some(status);
                        item.credentials = next_credentials;
                        item.notice = Some(Ok("MCP service state updated".to_string()));
                    }
                    Err(error) => item.notice = Some(Err(error)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn copy_configuration(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(status) = self.status.as_ref() else {
            return;
        };
        let Some(endpoint) = status.endpoint.as_ref() else {
            return;
        };
        let Some(credentials) = self.credentials.as_ref() else {
            self.notice = Some(Err(
                "Start or restart the MCP service in this session before copying its token"
                    .to_string(),
            ));
            cx.notify();
            return;
        };
        let config = serde_json::json!({
            "mcpServers": {
                "astesia": {
                    "type": "streamable-http",
                    "url": endpoint,
                    "headers": {
                        "Authorization": format!("Bearer {}", credentials.auth_token),
                    }
                }
            }
        });
        match serde_json::to_string_pretty(&config) {
            Ok(config) => {
                cx.write_to_clipboard(ClipboardItem::new_string(config));
                self.notice = Some(Ok("MCP client configuration copied".to_string()));
            }
            Err(error) => self.notice = Some(Err(error.to_string())),
        }
        cx.notify();
    }
}

enum McpOperation {
    Start(McpCredentials),
    Stop,
    Restart(McpCredentials),
}

impl McpOperation {
    fn credentials(&self) -> Option<&McpCredentials> {
        match self {
            Self::Start(credentials) | Self::Restart(credentials) => Some(credentials),
            Self::Stop => None,
        }
    }
}

impl Render for McpServiceItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let language = self.settings.read(cx).language();
        let status = self.status.clone();
        let running = status
            .as_ref()
            .is_some_and(|status| status.state == McpServicePhase::Running);
        let unavailable = status.as_ref().is_some_and(|status| !status.available);
        let phase = status
            .as_ref()
            .map(|status| phase_label(status.state, language))
            .unwrap_or_else(|| text(language, "正在加载", "Loading"));

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("McpServiceItem")
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(44.0))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Label::new("MCP Server")
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::MEDIUM),
                    )
                    .child(Label::new(phase).size(LabelSize::XSmall).color(if running {
                        Color::Success
                    } else {
                        Color::Muted
                    }))
                    .child(div().flex_1())
                    .child(
                        Button::new("refresh-mcp-status", text(language, "刷新", "Refresh"))
                            .size(ButtonSize::Compact)
                            .disabled(self.busy)
                            .on_click(cx.listener(Self::refresh_click)),
                    ),
            )
            .child(
                v_flex()
                    .p_4()
                    .gap_3()
                    .child(status_field(
                        text(language, "传输", "Transport"),
                        status.as_ref().map(|status| status.transport).unwrap_or("—"),
                    ))
                    .child(status_field(
                        text(language, "端点", "Endpoint"),
                        status
                            .as_ref()
                            .and_then(|status| status.endpoint.as_deref())
                            .unwrap_or("—"),
                    ))
                    .child(status_field(
                        "PID",
                        status
                            .as_ref()
                            .and_then(|status| status.pid)
                            .map(|pid| pid.to_string())
                            .unwrap_or_else(|| "—".to_string()),
                    ))
                    .child(status_field(
                        text(language, "二进制", "Binary"),
                        status
                            .as_ref()
                            .and_then(|status| status.binary_path.as_deref())
                            .unwrap_or("—"),
                    ))
                    .when(unavailable, |element| {
                        element.child(
                            Label::new(text(
                                language,
                                "MCP sidecar 未安装。请先构建或安装目标平台 sidecar。",
                                "The MCP sidecar is not installed. Build or install the target-platform sidecar first.",
                            ))
                            .size(LabelSize::XSmall)
                            .color(Color::Warning),
                        )
                    })
                    .when_some(
                        status.as_ref().and_then(|status| status.last_error.clone()),
                        |element, error| {
                            element.child(Label::new(error).size(LabelSize::XSmall).color(Color::Error))
                        },
                    )
                    .when_some(self.notice.clone(), |element, notice| {
                        let (message, color) = match notice {
                            Ok(message) => (message, Color::Success),
                            Err(message) => (message, Color::Error),
                        };
                        element.child(Label::new(message).size(LabelSize::XSmall).color(color))
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("start-mcp-service", text(language, "启动", "Start"))
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Filled)
                                    .loading(self.busy && !running)
                                    .disabled(self.busy || running || unavailable)
                                    .on_click(cx.listener(Self::start)),
                            )
                            .child(
                                Button::new("stop-mcp-service", text(language, "停止", "Stop"))
                                    .size(ButtonSize::Compact)
                                    .disabled(self.busy || !running)
                                    .on_click(cx.listener(Self::stop)),
                            )
                            .child(
                                Button::new("restart-mcp-service", text(language, "重启", "Restart"))
                                    .size(ButtonSize::Compact)
                                    .disabled(self.busy || !running || unavailable)
                                    .on_click(cx.listener(Self::restart)),
                            )
                            .child(
                                Button::new(
                                    "copy-mcp-configuration",
                                    text(language, "复制配置", "Copy Configuration"),
                                )
                                .size(ButtonSize::Compact)
                                .disabled(!running || self.credentials.is_none())
                                .on_click(cx.listener(Self::copy_configuration)),
                            ),
                    )
                    .child(
                        Label::new(text(
                            language,
                            "每次启动或重启都会选择新的本地端口和认证令牌。令牌仅在复制客户端配置时显示。",
                            "Every start or restart chooses a new local port and authentication token. The token is exposed only when copying client configuration.",
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    ),
            )
    }
}

fn generate_credentials() -> Result<McpCredentials, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("Could not choose an MCP port: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("Could not read the MCP port: {error}"))?
        .port();
    drop(listener);
    let mut bytes = [0_u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Could not generate an MCP authentication token".to_string())?;
    let auth_token = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(McpCredentials { port, auth_token })
}

fn phase_label(phase: McpServicePhase, language: crate::platform::UiLanguage) -> &'static str {
    match phase {
        McpServicePhase::Stopped => text(language, "已停止", "Stopped"),
        McpServicePhase::Starting => text(language, "启动中", "Starting"),
        McpServicePhase::Running => text(language, "运行中", "Running"),
        McpServicePhase::Stopping => text(language, "停止中", "Stopping"),
        McpServicePhase::Error => text(language, "错误", "Error"),
    }
}

fn status_field(label: impl Into<SharedString>, value: impl Into<SharedString>) -> AnyElement {
    h_flex()
        .gap_2()
        .child(
            div().w(px(120.0)).child(
                Label::new(label)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            ),
        )
        .child(Label::new(value).size(LabelSize::XSmall))
        .into_any_element()
}
