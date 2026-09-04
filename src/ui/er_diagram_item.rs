use std::{collections::HashMap, sync::Arc};

use gpui::{
    canvas, fill, point, px, BorderStyle, Bounds, ClickEvent, Entity, FocusHandle, Hsla,
    MouseButton, MouseDownEvent, MouseMoveEvent, PathBuilder, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, Subscription,
};
use zed_ui::{prelude::*, Tooltip};

use crate::application::{
    Application, ErBounds, ErDiagramState, ErLayout, ErLoadError, ErPoint, ErSchema, ErStatus,
    QueryTarget,
};
use crate::db::TableRef;

use super::{localization::text, shell::ShellSettings};

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 2.5;
const ZOOM_STEP: f32 = 0.15;

struct OverviewPaint {
    nodes: Vec<(ErPoint, f32, f32)>,
    layout_width: f32,
    layout_height: f32,
    background: Hsla,
    node_color: Hsla,
    viewport_color: Hsla,
    viewport_origin: ErPoint,
    viewport_extent: (f32, f32),
}

enum DragState {
    Pan {
        start: Point<Pixels>,
        origin: ErPoint,
    },
    Node {
        table: usize,
        start: Point<Pixels>,
        origin: ErPoint,
    },
}

pub(super) struct ErDiagramItem {
    application: Arc<Application>,
    state: ErDiagramState,
    layout: ErLayout,
    offsets: Vec<ErPoint>,
    pan: ErPoint,
    zoom: f32,
    drag: Option<DragState>,
    focus_handle: FocusHandle,
    settings: Entity<ShellSettings>,
    _settings_observation: Subscription,
}

impl ErDiagramItem {
    pub(super) fn new(
        application: Arc<Application>,
        target: QueryTarget,
        settings: Entity<ShellSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let mut item = Self {
            application,
            state: ErDiagramState::new(target),
            layout: ErLayout::default(),
            offsets: Vec::new(),
            pan: ErPoint { x: 24.0, y: 24.0 },
            zoom: 1.0,
            drag: None,
            focus_handle: cx.focus_handle(),
            settings,
            _settings_observation: settings_observation,
        };
        item.refresh(cx);
        item
    }

    pub(super) fn label(&self, cx: &App) -> String {
        let language = self.settings.read(cx).language();
        format!(
            "{} [{}]",
            text(language, "关系图", "ER Diagram"),
            self.state.target().database
        )
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    pub(super) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<Self>,
    ) {
        let language = self.settings.read(cx).language();
        if self.state.invalidate_session(
            connection_id,
            session_generation,
            text(
                language,
                "连接会话已更改。请从侧边栏重新打开关系图。",
                "The connection session changed. Reopen the ER diagram from the sidebar.",
            ),
        ) {
            self.drag = None;
            cx.notify();
        }
    }

    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.state.begin_load() else {
            return;
        };
        cx.notify();
        let service = self.application.er_diagrams().clone();
        let target = self.state.target().clone();
        let load = gpui_tokio::Tokio::spawn(cx, async move { service.load(&target).await });
        cx.spawn(async move |item, cx| {
            let result = match load.await {
                Ok(result) => result,
                Err(error) => Err(ErLoadError::BackgroundTask(error.to_string())),
            };
            item.update(cx, |item, cx| {
                let schema = result.as_ref().ok().cloned();
                if item.state.finish_load(request, result) {
                    if let Some(schema) = schema {
                        item.layout = ErLayout::build(&schema);
                        item.offsets = vec![ErPoint { x: 0.0, y: 0.0 }; schema.tables.len()];
                        item.pan = ErPoint { x: 24.0, y: 24.0 };
                        item.zoom = 1.0;
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn refresh_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }

    fn zoom_in(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(self.zoom + ZOOM_STEP, cx);
    }

    fn zoom_out(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(self.zoom - ZOOM_STEP, cx);
    }

    fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if (zoom - self.zoom).abs() > f32::EPSILON {
            self.zoom = zoom;
            cx.notify();
        }
    }

    fn fit(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.layout.bounds(&self.offsets) else {
            return;
        };
        let viewport = window.viewport_size();
        let available_width = (f32::from(viewport.width) - 320.0).max(320.0);
        let available_height = (f32::from(viewport.height) - 150.0).max(240.0);
        self.zoom = ((available_width - 48.0) / bounds.width.max(1.0))
            .min((available_height - 48.0) / bounds.height.max(1.0))
            .clamp(MIN_ZOOM, 1.5);
        self.pan = ErPoint {
            x: 24.0 - bounds.origin.x * self.zoom,
            y: 24.0 - bounds.origin.y * self.zoom,
        };
        cx.notify();
    }

    fn begin_pan(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.drag = Some(DragState::Pan {
            start: event.position,
            origin: self.pan,
        });
        cx.stop_propagation();
    }

    fn begin_node_drag(
        &mut self,
        table: usize,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = self.node_offset(table);
        self.drag = Some(DragState::Node {
            table,
            start: event.position,
            origin,
        });
        cx.stop_propagation();
    }

    fn drag_pointer(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = &self.drag else {
            return;
        };
        let start = match drag {
            DragState::Pan { start, .. } | DragState::Node { start, .. } => *start,
        };
        let delta_x = f32::from(event.position.x - start.x);
        let delta_y = f32::from(event.position.y - start.y);
        match *drag {
            DragState::Pan { origin, .. } => {
                self.pan = ErPoint {
                    x: origin.x + delta_x,
                    y: origin.y + delta_y,
                };
            }
            DragState::Node { table, origin, .. } => {
                if let Some(offset) = self.offsets.get_mut(table) {
                    *offset = ErPoint {
                        x: origin.x + delta_x / self.zoom,
                        y: origin.y + delta_y / self.zoom,
                    };
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.stop_propagation();
        }
    }

    fn scroll_zoom(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ticks = match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => f32::from(pixels.y) / 40.0,
        }
        .clamp(-1.0, 1.0);
        if ticks != 0.0 {
            self.set_zoom(self.zoom - ticks * ZOOM_STEP, cx);
        }
        cx.stop_propagation();
    }

    fn render_schema(
        &self,
        schema: &ErSchema,
        language: crate::platform::UiLanguage,
        viewport_width: f32,
        viewport_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if schema.tables.is_empty() {
            return empty_state(
                text(language, "没有可显示的表", "No tables to diagram"),
                text(
                    language,
                    "此数据库未返回任何关系表。刷新或选择其他数据库。",
                    "This database returned no relational tables. Refresh or choose another database.",
                ),
                cx,
            );
        }
        let colors = cx.theme().colors().clone();
        let positions = self
            .layout
            .nodes
            .iter()
            .map(|node| {
                let offset = self.node_offset(node.table);
                ErPoint {
                    x: self.pan.x + (node.position.x + offset.x) * self.zoom,
                    y: self.pan.y + (node.position.y + offset.y) * self.zoom,
                }
            })
            .collect::<Vec<_>>();
        let node_by_table = schema
            .tables
            .iter()
            .enumerate()
            .map(|(index, table)| (table.reference.clone(), index))
            .collect::<HashMap<TableRef, usize>>();
        let edges = schema
            .relationships
            .iter()
            .filter_map(|relationship| {
                let from = *node_by_table.get(&relationship.from_table)?;
                let to = *node_by_table.get(&relationship.to_table)?;
                let to_node = self.layout.nodes.get(to)?;
                let from_column = relationship
                    .from_columns
                    .first()
                    .and_then(|name| {
                        schema.tables[from]
                            .columns
                            .iter()
                            .position(|column| &column.name == name)
                    })
                    .unwrap_or(schema.tables[from].columns.len().min(12) / 2)
                    .min(11);
                let to_column = relationship
                    .to_columns
                    .first()
                    .and_then(|name| {
                        schema.tables[to]
                            .columns
                            .iter()
                            .position(|column| &column.name == name)
                    })
                    .unwrap_or(schema.tables[to].columns.len().min(12) / 2)
                    .min(11);
                Some((
                    ErPoint {
                        x: positions[from].x,
                        y: positions[from].y + (48.0 + from_column as f32 * 20.0) * self.zoom,
                    },
                    ErPoint {
                        x: positions[to].x + to_node.width * self.zoom,
                        y: positions[to].y + (48.0 + to_column as f32 * 20.0) * self.zoom,
                    },
                ))
            })
            .collect::<Vec<_>>();
        let bounds = self.layout.bounds(&self.offsets).unwrap_or(ErBounds {
            origin: ErPoint { x: 0.0, y: 0.0 },
            width: self.layout.width,
            height: self.layout.height,
        });
        let overview_nodes = self
            .layout
            .nodes
            .iter()
            .map(|node| {
                let offset = self.node_offset(node.table);
                (
                    ErPoint {
                        x: node.position.x + offset.x - bounds.origin.x,
                        y: node.position.y + offset.y - bounds.origin.y,
                    },
                    node.width,
                    node.height,
                )
            })
            .collect::<Vec<_>>();
        let edge_color = colors.text_muted.alpha(0.72);
        let layout_width = bounds.width;
        let layout_height = bounds.height;
        let viewport_right =
            ((viewport_width - self.pan.x) / self.zoom - bounds.origin.x).clamp(0.0, layout_width);
        let viewport_bottom = (((viewport_height - 80.0).max(1.0) - self.pan.y) / self.zoom
            - bounds.origin.y)
            .clamp(0.0, layout_height);
        let viewport_origin = ErPoint {
            x: (-self.pan.x / self.zoom - bounds.origin.x).clamp(0.0, layout_width),
            y: (-self.pan.y / self.zoom - bounds.origin.y).clamp(0.0, layout_height),
        };
        let viewport_extent = (
            (viewport_right - viewport_origin.x).max(0.0),
            (viewport_bottom - viewport_origin.y).max(0.0),
        );
        let overview = OverviewPaint {
            nodes: overview_nodes,
            layout_width,
            layout_height,
            background: colors.panel_background,
            node_color: colors.text_muted,
            viewport_color: colors.text_accent,
            viewport_origin,
            viewport_extent,
        };

        div()
            .id("er-diagram-canvas")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .cursor_grab()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_pan))
            .on_mouse_move(cx.listener(Self::drag_pointer))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|item, _, _, cx| item.end_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|item, _, _, cx| item.end_drag(cx)),
            )
            .on_scroll_wheel(cx.listener(Self::scroll_zoom))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| paint_edges(bounds, &edges, edge_color, window),
                )
                .absolute()
                .inset_0(),
            )
            .children(self.layout.nodes.iter().filter_map(|node| {
                let table = schema.tables.get(node.table)?;
                let position = positions[node.table];
                let table_index = node.table;
                Some(
                    v_flex()
                        .id(("er-table-node", table_index))
                        .absolute()
                        .left(px(position.x))
                        .top(px(position.y))
                        .w(px(node.width * self.zoom))
                        .h(px(node.height * self.zoom))
                        .overflow_hidden()
                        .cursor_move()
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.panel_background)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |item, event, window, cx| {
                                item.begin_node_drag(table_index, event, window, cx);
                            }),
                        )
                        .child(
                            v_flex()
                                .min_h(px(38.0 * self.zoom))
                                .flex_none()
                                .justify_center()
                                .px_2()
                                .border_b_1()
                                .border_color(colors.border)
                                .child(
                                    Label::new(table.reference.name().to_string())
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::SEMIBOLD)
                                        .truncate(),
                                )
                                .children(table.reference.schema().map(|schema| {
                                    Label::new(schema.to_string())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate()
                                })),
                        )
                        .children(table.columns.iter().take(12).map(|column| {
                            h_flex()
                                .h(px(20.0 * self.zoom))
                                .flex_none()
                                .gap_1()
                                .px_2()
                                .child(
                                    Label::new(if column.is_primary_key { "PK" } else { "" })
                                        .size(LabelSize::XSmall)
                                        .color(Color::Accent),
                                )
                                .child(
                                    Label::new(column.name.clone())
                                        .flex_1()
                                        .size(LabelSize::XSmall)
                                        .truncate(),
                                )
                                .child(
                                    Label::new(column.data_type.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                )
                        }))
                        .children((table.columns.len() > 12).then(|| {
                            Label::new(format!(
                                "+{} {}",
                                table.columns.len() - 12,
                                text(language, "列", "columns")
                            ))
                            .mx_2()
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                        })),
                )
            }))
            .child(
                div()
                    .absolute()
                    .right(px(12.0))
                    .bottom(px(12.0))
                    .w(px(164.0))
                    .h(px(104.0))
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, _| {
                                paint_overview(bounds, &overview, window);
                            },
                        )
                        .size_full(),
                    ),
            )
            .into_any_element()
    }

    fn node_offset(&self, table: usize) -> ErPoint {
        self.offsets
            .get(table)
            .copied()
            .unwrap_or(ErPoint { x: 0.0, y: 0.0 })
    }
}

impl Render for ErDiagramItem {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let language = self.settings.read(cx).language();
        let target = self.state.target();
        let (loading, error, schema, unavailable) = match self.state.status() {
            ErStatus::Idle => (false, None, None, None),
            ErStatus::Loading(schema) => (true, None, schema, None),
            ErStatus::Ready(schema) => (false, None, Some(schema), None),
            ErStatus::Failed(error, schema) => (false, Some(error.to_string()), schema, None),
            ErStatus::Unavailable(reason) => (false, None, None, Some(reason)),
        };
        let viewport = window.viewport_size();
        let content = if let Some(schema) = schema {
            self.render_schema(
                schema,
                language,
                f32::from(viewport.width),
                f32::from(viewport.height),
                cx,
            )
        } else if let Some(reason) = unavailable {
            empty_state(
                text(language, "关系图不可用", "ER diagram unavailable"),
                reason,
                cx,
            )
        } else if loading {
            empty_state(
                text(language, "正在加载关系图…", "Loading ER diagram…"),
                text(
                    language,
                    "正在读取表、列和外键。",
                    "Reading tables, columns, and foreign keys.",
                ),
                cx,
            )
        } else {
            empty_state(
                text(language, "关系图尚未加载", "ER diagram is not loaded"),
                text(language, "刷新以重试。", "Refresh to try again."),
                cx,
            )
        };

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("ErDiagramItem")
            .size_full()
            .overflow_hidden()
            .bg(colors.background)
            .child(
                h_flex()
                    .h(px(40.0))
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Label::new(text(language, "实体关系图", "Entity Relationship Diagram"))
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(format!("{} · {}", target.connection_name, target.database))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .truncate(),
                    )
                    .child(div().flex_1())
                    .child(
                        Label::new(format!("{}%", (self.zoom * 100.0).round() as u32))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        IconButton::new("er-zoom-out", IconName::SquareMinus)
                            .icon_size(IconSize::XSmall)
                            .tooltip(Tooltip::text(text(language, "缩小", "Zoom out")))
                            .disabled(self.zoom <= MIN_ZOOM)
                            .on_click(cx.listener(Self::zoom_out)),
                    )
                    .child(
                        IconButton::new("er-zoom-in", IconName::SquarePlus)
                            .icon_size(IconSize::XSmall)
                            .tooltip(Tooltip::text(text(language, "放大", "Zoom in")))
                            .disabled(self.zoom >= MAX_ZOOM)
                            .on_click(cx.listener(Self::zoom_in)),
                    )
                    .child(
                        Button::new("er-fit", text(language, "适合窗口", "Fit"))
                            .size(ButtonSize::Compact)
                            .disabled(schema.is_none())
                            .on_click(cx.listener(Self::fit)),
                    )
                    .child(
                        Button::new("er-refresh", text(language, "刷新", "Refresh"))
                            .size(ButtonSize::Compact)
                            .loading(loading)
                            .disabled(loading || unavailable.is_some())
                            .on_click(cx.listener(Self::refresh_click)),
                    ),
            )
            .children(error.map(|error| {
                h_flex()
                    .min_h(px(30.0))
                    .flex_none()
                    .gap_2()
                    .px_3()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Icon::new(IconName::Warning)
                            .size(IconSize::XSmall)
                            .color(Color::Error),
                    )
                    .child(
                        Label::new(error)
                            .size(LabelSize::XSmall)
                            .color(Color::Error),
                    )
            }))
            .child(content)
    }
}

fn paint_edges(
    bounds: Bounds<Pixels>,
    edges: &[(ErPoint, ErPoint)],
    color: Hsla,
    window: &mut Window,
) {
    for (from, to) in edges {
        let start = point(bounds.origin.x + px(from.x), bounds.origin.y + px(from.y));
        let end = point(bounds.origin.x + px(to.x), bounds.origin.y + px(to.y));
        let bend = ((end.x - start.x).abs() / 2.0).max(px(24.0));
        let mut path = PathBuilder::stroke(px(1.5));
        path.move_to(start);
        path.cubic_bezier_to(
            end,
            point(start.x - bend, start.y),
            point(end.x + bend, end.y),
        );
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}

fn paint_overview(bounds: Bounds<Pixels>, overview: &OverviewPaint, window: &mut Window) {
    window.paint_quad(fill(bounds, overview.background));
    if overview.layout_width <= 0.0 || overview.layout_height <= 0.0 {
        return;
    }
    let scale = ((f32::from(bounds.size.width) - 12.0) / overview.layout_width)
        .min((f32::from(bounds.size.height) - 12.0) / overview.layout_height);
    for (position, width, height) in &overview.nodes {
        window.paint_quad(fill(
            Bounds::new(
                point(
                    bounds.origin.x + px(6.0 + position.x * scale),
                    bounds.origin.y + px(6.0 + position.y * scale),
                ),
                gpui::size(px(width * scale), px(height * scale)),
            ),
            overview.node_color.alpha(0.55),
        ));
    }
    let viewport_bounds = Bounds::new(
        point(
            bounds.origin.x + px(6.0 + overview.viewport_origin.x * scale),
            bounds.origin.y + px(6.0 + overview.viewport_origin.y * scale),
        ),
        gpui::size(
            px((overview.viewport_extent.0 * scale).max(2.0)),
            px((overview.viewport_extent.1 * scale).max(2.0)),
        ),
    );
    window.paint_quad(gpui::quad(
        viewport_bounds,
        px(0.0),
        gpui::transparent_black(),
        px(1.0),
        overview.viewport_color,
        BorderStyle::Solid,
    ));
}

fn empty_state(
    title: impl Into<gpui::SharedString>,
    detail: impl Into<gpui::SharedString>,
    cx: &mut Context<ErDiagramItem>,
) -> AnyElement {
    v_flex()
        .flex_1()
        .justify_center()
        .items_center()
        .gap_1()
        .p_4()
        .child(Label::new(title.into()).size(LabelSize::Small))
        .child(
            Label::new(detail.into())
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .bg(cx.theme().colors().background)
        .into_any_element()
}
