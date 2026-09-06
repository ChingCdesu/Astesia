use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::ui::components::{prelude::*, Tooltip};
use gpui_kit::{
    canvas, fill, point, px, BorderStyle, Bounds, ClickEvent, Entity, FocusHandle, Hsla,
    MouseButton, MouseDownEvent, MouseMoveEvent, PathBuilder, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, Subscription,
};

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
        let load = crate::ui::runtime::spawn(cx, async move { service.load(&target).await });
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
        let colors = cx.theme().colors();
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
                let from_node = self.layout.nodes.get(from)?;
                let to_node = self.layout.nodes.get(to)?;
                let target_to_right = positions[from].x + from_node.width * self.zoom / 2.0
                    < positions[to].x + to_node.width * self.zoom / 2.0;
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
                        x: positions[from].x
                            + if target_to_right {
                                from_node.width * self.zoom
                            } else {
                                0.0
                            },
                        y: positions[from].y
                            + (ErLayout::HEADER_HEIGHT
                                + (from_column as f32 + 0.5) * ErLayout::ROW_HEIGHT)
                                * self.zoom,
                    },
                    ErPoint {
                        x: positions[to].x
                            + if target_to_right {
                                0.0
                            } else {
                                to_node.width * self.zoom
                            },
                        y: positions[to].y
                            + (ErLayout::HEADER_HEIGHT
                                + (to_column as f32 + 0.5) * ErLayout::ROW_HEIGHT)
                                * self.zoom,
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
        let foreign_columns = schema
            .relationships
            .iter()
            .flat_map(|relationship| {
                relationship
                    .from_columns
                    .iter()
                    .map(|column| (relationship.from_table.clone(), column.clone()))
            })
            .collect::<HashSet<_>>();
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
                        .bg(colors.surface_background)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |item, event, window, cx| {
                                item.begin_node_drag(table_index, event, window, cx);
                            }),
                        )
                        .child(
                            h_flex()
                                .h(px(ErLayout::HEADER_HEIGHT * self.zoom))
                                .flex_none()
                                .px(px(12.0 * self.zoom))
                                .border_b_1()
                                .border_color(colors.border)
                                .child(
                                    Label::new(table.reference.to_string())
                                        .size(LabelSize::Custom(
                                            crate::ui::components::TextSize::Small.rems(cx)
                                                * self.zoom,
                                        ))
                                        .truncate(),
                                ),
                        )
                        .children(table.columns.iter().take(12).map(|column| {
                            let foreign = foreign_columns
                                .contains(&(table.reference.clone(), column.name.clone()));
                            h_flex()
                                .h(px(ErLayout::ROW_HEIGHT * self.zoom))
                                .flex_none()
                                .bg(colors.editor_background)
                                .border_b_1()
                                .border_color(colors.border)
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .px(px(12.0 * self.zoom))
                                        .gap(px(4.0 * self.zoom))
                                        .child(
                                            div().w(px(24.0 * self.zoom)).flex_none().child(
                                                Label::new(if column.is_primary_key {
                                                    "PK"
                                                } else if foreign {
                                                    "FK"
                                                } else {
                                                    ""
                                                })
                                                .buffer_font(cx)
                                                .size(LabelSize::Custom(
                                                    crate::ui::components::TextSize::XSmall
                                                        .rems(cx)
                                                        * self.zoom,
                                                )),
                                            ),
                                        )
                                        .child(
                                            Label::new(column.name.clone())
                                                .buffer_font(cx)
                                                .size(LabelSize::Custom(
                                                    crate::ui::components::TextSize::XSmall
                                                        .rems(cx)
                                                        * self.zoom,
                                                ))
                                                .truncate(),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(110.0 * self.zoom))
                                        .h_full()
                                        .flex_none()
                                        .px(px(8.0 * self.zoom))
                                        .border_l_1()
                                        .border_color(colors.border)
                                        .flex()
                                        .items_center()
                                        .child(
                                            Label::new(column.data_type.clone())
                                                .buffer_font(cx)
                                                .size(LabelSize::Custom(
                                                    crate::ui::components::TextSize::XSmall
                                                        .rems(cx)
                                                        * self.zoom,
                                                ))
                                                .color(Color::Muted)
                                                .truncate(),
                                        ),
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
                div().absolute().left(px(20.0)).bottom(px(20.0)).child(
                    Label::new(text(
                        language,
                        "拖动表调整位置 · 拖动画布平移 · 连线 FK → PK",
                        "Drag tables to arrange · Drag canvas to pan · Links FK → PK",
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                ),
            )
            .child(
                div()
                    .absolute()
                    .right(px(12.0))
                    .bottom(px(12.0))
                    .w(px(164.0))
                    .h(px(104.0))
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.surface_background)
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
        let colors = cx.theme().colors();
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
        let content = if let Some(schema) =
            schema.filter(|schema| error.is_none() || !schema.tables.is_empty())
        {
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
        } else if error.is_some() {
            empty_state(
                text(language, "无法加载关系图", "Unable to load ER diagram"),
                text(
                    language,
                    "请检查连接状态和读取表结构的权限后重试。",
                    "Check the connection and schema permissions, then retry.",
                ),
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
            .bg(colors.editor_background)
            .child(
                h_flex()
                    .h(DynamicSpacing::Base32.rems(cx))
                    .flex_none()
                    .gap(DynamicSpacing::Base04.rems(cx))
                    .px(DynamicSpacing::Base08.rems(cx))
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.surface_background)
                    .child(super::button::button_with_disabled_state(
                        crate::ui::components::ButtonLike::new("er-refresh", "")
                            .size(ButtonSize::Compact)
                            .child(
                                Icon::new(if loading {
                                    IconName::LoaderCircle
                                } else {
                                    IconName::RotateCw
                                })
                                .size(IconSize::Small),
                            )
                            .aria_label(text(language, "刷新", "Refresh"))
                            .tooltip(Tooltip::text(text(language, "刷新", "Refresh")))
                            .on_click(cx.listener(Self::refresh_click)),
                        loading || unavailable.is_some(),
                        window,
                        cx,
                    ))
                    .child(super::button::button_with_disabled_state(
                        crate::ui::components::ButtonLike::new("er-zoom-out", "")
                            .size(ButtonSize::Compact)
                            .aria_label(text(language, "缩小", "Zoom out"))
                            .tooltip(Tooltip::text(text(language, "缩小", "Zoom out")))
                            .child(Label::new("−").size(LabelSize::Small))
                            .on_click(cx.listener(Self::zoom_out)),
                        self.zoom <= MIN_ZOOM
                            || schema.is_none_or(|schema| schema.tables.is_empty()),
                        window,
                        cx,
                    ))
                    .child(
                        Label::new(format!("{}%", (self.zoom * 100.0).round() as u32))
                            .size(LabelSize::XSmall),
                    )
                    .child(super::button::button_with_disabled_state(
                        crate::ui::components::ButtonLike::new("er-zoom-in", "")
                            .size(ButtonSize::Compact)
                            .child(Icon::new(IconName::Plus).size(IconSize::XSmall))
                            .aria_label(text(language, "放大", "Zoom in"))
                            .tooltip(Tooltip::text(text(language, "放大", "Zoom in")))
                            .on_click(cx.listener(Self::zoom_in)),
                        self.zoom >= MAX_ZOOM
                            || schema.is_none_or(|schema| schema.tables.is_empty()),
                        window,
                        cx,
                    ))
                    .child(super::button::button_with_disabled_state(
                        crate::ui::components::ButtonLike::new("er-fit", "")
                            .size(ButtonSize::Compact)
                            .aria_label(text(language, "适合窗口", "Fit to Window"))
                            .tooltip(Tooltip::text(text(language, "适合窗口", "Fit to Window")))
                            .child(
                                Icon::from_path("icons/astesia/fit-window.svg")
                                    .size(IconSize::Small),
                            )
                            .on_click(cx.listener(Self::fit)),
                        schema.is_none_or(|schema| schema.tables.is_empty()),
                        window,
                        cx,
                    ))
                    .child(div().flex_1())
                    .children(schema.map(|schema| {
                        Label::new(format!(
                            "{} {} · {} {}",
                            schema.tables.len(),
                            text(language, "张表", "tables"),
                            schema.relationships.len(),
                            text(language, "条外键关系", "foreign keys")
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                    }))
                    .child(
                        Label::new(format!("{} / {}", target.connection_name, target.database))
                            .size(LabelSize::XSmall)
                            .truncate(),
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
                    .bg(colors.surface_background)
                    .child(
                        Icon::new(IconName::TriangleAlert)
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
        let arrow_offset = if end.x >= start.x { px(-6.0) } else { px(6.0) };
        let mut path = PathBuilder::stroke(px(1.5));
        path.move_to(start);
        let middle = (start.x + end.x) / 2.0;
        path.line_to(point(middle, start.y));
        path.line_to(point(middle, end.y));
        path.line_to(end);
        path.move_to(point(end.x + arrow_offset, end.y - px(5.0)));
        path.line_to(end);
        path.line_to(point(end.x + arrow_offset, end.y + px(5.0)));
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
                gpui_kit::size(px(width * scale), px(height * scale)),
            ),
            overview.node_color.alpha(0.55),
        ));
    }
    let viewport_bounds = Bounds::new(
        point(
            bounds.origin.x + px(6.0 + overview.viewport_origin.x * scale),
            bounds.origin.y + px(6.0 + overview.viewport_origin.y * scale),
        ),
        gpui_kit::size(
            px((overview.viewport_extent.0 * scale).max(2.0)),
            px((overview.viewport_extent.1 * scale).max(2.0)),
        ),
    );
    window.paint_quad(gpui_kit::quad(
        viewport_bounds,
        px(0.0),
        gpui_kit::transparent_black(),
        px(1.0),
        overview.viewport_color,
        BorderStyle::Solid,
    ));
}

fn empty_state(
    title: impl Into<gpui_kit::SharedString>,
    detail: impl Into<gpui_kit::SharedString>,
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
        .bg(cx.theme().colors().editor_background)
        .into_any_element()
}
