use gpui::prelude::*;
use gpui::{
    canvas, fill, point, px, Bounds, ClickEvent, Entity, FocusHandle, Focusable, Hsla, PathBuilder,
    Pixels,
};
use zed_ui::prelude::*;

use crate::application::{ChartDataError, ChartModel, ChartSeries, ChartType};
use crate::platform::UiLanguage;

use super::localization::text;
use super::shell::ShellSettings;

const SERIES_COLORS: [u32; 6] = [0x4f46e5, 0x0d9488, 0xd97706, 0xdc2626, 0x7c3aed, 0x0284c7];

pub(super) struct ChartView {
    model: ChartModel,
    focus_handle: FocusHandle,
    settings: Entity<ShellSettings>,
}

impl ChartView {
    pub(super) fn new(
        model: ChartModel,
        settings: Entity<ShellSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            model,
            focus_handle: cx.focus_handle(),
            settings,
        }
    }

    pub(super) fn replace_data(
        &mut self,
        columns: Vec<String>,
        rows: &[Vec<serde_json::Value>],
        cx: &mut Context<Self>,
    ) {
        self.model.replace_data(columns, rows);
        cx.notify();
    }

    fn select_chart_type(
        &mut self,
        chart_type: ChartType,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model.set_chart_type(chart_type) {
            cx.notify();
        }
    }

    fn select_x_column(
        &mut self,
        column: usize,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model.set_x_column(column) {
            cx.notify();
        }
    }

    fn toggle_y_column(
        &mut self,
        column: usize,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model.toggle_y_column(column) {
            cx.notify();
        }
    }
}

impl Focusable for ChartView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChartView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        let series = self.model.series();
        let chart_type = self.model.chart_type();
        let columns = self.model.columns().to_vec();
        let numeric_columns = self.model.numeric_columns().to_vec();
        let x_column = self.model.x_column();
        let y_columns = self.model.y_columns().to_vec();

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("ChartView")
            .size_full()
            .overflow_hidden()
            .bg(colors.background)
            .child(
                h_flex()
                    .id("chart-type-controls")
                    .h(px(36.0))
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .overflow_x_scroll()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Label::new(text(language, "图表类型", "Chart type"))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .children(ChartType::ALL.into_iter().map(|kind| {
                        Button::new(
                            format!("chart-type-{}", chart_type_id(kind)),
                            chart_type_label(kind, language),
                        )
                        .size(ButtonSize::Compact)
                        .toggle_state(chart_type == kind)
                        .on_click(cx.listener(
                            move |view, event, window, cx| {
                                view.select_chart_type(kind, event, window, cx);
                            },
                        ))
                    })),
            )
            .child(
                h_flex()
                    .id("chart-column-controls")
                    .h(px(36.0))
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .overflow_x_scroll()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Label::new("X")
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    )
                    .children(columns.iter().enumerate().map(|(column, name)| {
                        Button::new(format!("chart-x-column-{column}"), name.clone())
                            .size(ButtonSize::Compact)
                            .toggle_state(x_column == column)
                            .on_click(cx.listener(move |view, event, window, cx| {
                                view.select_x_column(column, event, window, cx);
                            }))
                    }))
                    .child(
                        Label::new("Y")
                            .ml_2()
                            .size(LabelSize::XSmall)
                            .weight(gpui::FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    )
                    .children(
                        columns
                            .iter()
                            .enumerate()
                            .filter(|(column, _)| numeric_columns[*column])
                            .map(|(column, name)| {
                                Button::new(format!("chart-y-column-{column}"), name.clone())
                                    .size(ButtonSize::Compact)
                                    .disabled(column == x_column)
                                    .toggle_state(y_columns.contains(&column))
                                    .on_click(cx.listener(move |view, event, window, cx| {
                                        view.toggle_y_column(column, event, window, cx);
                                    }))
                            }),
                    ),
            )
            .child(match series {
                Ok(series) => render_chart(series, chart_type, language, cx),
                Err(error) => render_error(error, language, cx),
            })
    }
}

fn render_chart(
    series: Vec<ChartSeries>,
    chart_type: ChartType,
    language: UiLanguage,
    cx: &mut Context<ChartView>,
) -> AnyElement {
    let colors = cx.theme().colors();
    let first_label = series
        .first()
        .and_then(|series| series.points.first())
        .map(|point| point.label.clone())
        .unwrap_or_default();
    let last_label = series
        .first()
        .and_then(|series| series.points.last())
        .map(|point| point.label.clone())
        .unwrap_or_default();
    let legend = chart_legend(&series, chart_type);
    let (minimum_y, maximum_y) = y_range(&series);
    let midpoint_y = minimum_y + (maximum_y - minimum_y) / 2.0;
    let grid = colors.border;
    let background = colors.background;

    v_flex()
        .flex_1()
        .min_h_0()
        .p_3()
        .gap_2()
        .child(
            h_flex()
                .id("chart-legend")
                .h(px(24.0))
                .flex_none()
                .items_center()
                .gap_3()
                .overflow_x_scroll()
                .child(
                    Label::new(format!(
                        "{} {}",
                        series
                            .iter()
                            .map(|series| series.points.len())
                            .max()
                            .unwrap_or(0),
                        text(language, "个数据点", "points")
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                )
                .children(legend),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_h(px(180.0))
                .border_1()
                .border_color(colors.border)
                .bg(colors.background)
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            paint_chart(bounds, &series, chart_type, grid, background, window);
                        },
                    )
                    .size_full(),
                )
                .when(chart_type != ChartType::Pie, |plot| {
                    plot.child(
                        v_flex()
                            .absolute()
                            .left(px(4.0))
                            .top(px(14.0))
                            .bottom(px(20.0))
                            .w(px(30.0))
                            .justify_between()
                            .items_end()
                            .children([maximum_y, midpoint_y, minimum_y].map(|value| {
                                Label::new(format_axis_value(value))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                            })),
                    )
                }),
        )
        .when(chart_type != ChartType::Pie, |chart| {
            chart.child(
                h_flex()
                    .h(px(18.0))
                    .flex_none()
                    .justify_between()
                    .child(
                        Label::new(first_label)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(last_label)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
        })
        .into_any_element()
}

fn chart_legend(series: &[ChartSeries], chart_type: ChartType) -> Vec<AnyElement> {
    if chart_type == ChartType::Pie {
        let total = series
            .first()
            .into_iter()
            .flat_map(|series| series.points.iter())
            .map(|point| point.y.max(0.0))
            .sum::<f64>();
        return series
            .first()
            .into_iter()
            .flat_map(|series| series.points.iter())
            .enumerate()
            .map(|(index, point)| {
                let percentage = if total > 0.0 {
                    point.y.max(0.0) / total * 100.0
                } else {
                    0.0
                };
                legend_item(index, format!("{} · {percentage:.0}%", point.label))
            })
            .collect();
    }
    series
        .iter()
        .enumerate()
        .map(|(index, series)| legend_item(index, series.name.clone()))
        .collect()
}

fn legend_item(index: usize, label: String) -> AnyElement {
    h_flex()
        .gap_1()
        .child(
            div()
                .size(px(7.0))
                .rounded_full()
                .bg(gpui::rgb(SERIES_COLORS[index % SERIES_COLORS.len()])),
        )
        .child(Label::new(label).size(LabelSize::XSmall))
        .into_any_element()
}

fn format_axis_value(value: f64) -> String {
    let absolute = value.abs();
    if absolute >= 1_000_000.0 {
        format!("{:.1}m", value / 1_000_000.0)
    } else if absolute >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else if value.fract().abs() < 0.01 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn render_error(
    error: ChartDataError,
    language: UiLanguage,
    _cx: &mut Context<ChartView>,
) -> AnyElement {
    let (title, detail) = match error {
        ChartDataError::Empty => (
            text(language, "没有可绘制的数据", "No chartable data"),
            text(
                language,
                "刷新结果或选择包含数据的结果集。",
                "Refresh or choose a result set with rows.",
            ),
        ),
        ChartDataError::NoNumericColumns => (
            text(language, "没有数值列", "No numeric columns"),
            text(
                language,
                "图表至少需要一个可映射到 Y 轴的数值列。",
                "Charts need at least one numeric column for the Y axis.",
            ),
        ),
        ChartDataError::NoSeriesSelected => (
            text(language, "未选择数值序列", "No numeric series selected"),
            text(
                language,
                "在上方选择一个或多个 Y 列。",
                "Select one or more Y columns above.",
            ),
        ),
        ChartDataError::ScatterRequiresNumericX => (
            text(
                language,
                "散点图需要数值 X 轴",
                "Scatter needs a numeric X axis",
            ),
            text(
                language,
                "选择一个数值列作为 X 轴，或切换图表类型。",
                "Choose a numeric X column or switch chart type.",
            ),
        ),
    };
    v_flex()
        .flex_1()
        .justify_center()
        .items_center()
        .gap_1()
        .p_4()
        .child(Label::new(title).size(LabelSize::Small))
        .child(
            Label::new(detail)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .into_any_element()
}

fn paint_chart(
    bounds: Bounds<Pixels>,
    series: &[ChartSeries],
    chart_type: ChartType,
    grid: Hsla,
    background: Hsla,
    window: &mut Window,
) {
    window.paint_quad(fill(bounds, background));
    if chart_type == ChartType::Pie {
        paint_pie(bounds, series, window);
        return;
    }
    let plot = inset_bounds(bounds, 36.0, 18.0, 18.0, 26.0);
    for step in 0..=4 {
        let y = plot.origin.y + plot.size.height * step as f32 / 4.0;
        window.paint_quad(fill(
            Bounds::new(
                point(plot.origin.x, y),
                gpui::size(plot.size.width, px(1.0)),
            ),
            grid,
        ));
    }
    match chart_type {
        ChartType::Bar => paint_bars(plot, series, window),
        ChartType::Line => paint_lines(plot, series, false, window),
        ChartType::Area => paint_lines(plot, series, true, window),
        ChartType::Scatter => paint_scatter(plot, series, window),
        ChartType::Pie => unreachable!(),
    }
}

fn paint_bars(bounds: Bounds<Pixels>, series: &[ChartSeries], window: &mut Window) {
    let (minimum, maximum) = y_range(series);
    let count = series
        .iter()
        .map(|series| series.points.len())
        .max()
        .unwrap_or(1);
    let group_width = bounds.size.width / count.max(1) as f32;
    let bar_width = (group_width * 0.72 / series.len().max(1) as f32).max(px(2.0));
    let baseline = y_position(0.0, minimum, maximum, bounds);
    for (series_index, series) in series.iter().enumerate() {
        let color = gpui::rgb(SERIES_COLORS[series_index % SERIES_COLORS.len()]);
        for (point_index, data) in series.points.iter().enumerate() {
            let y = y_position(data.y, minimum, maximum, bounds);
            let x = bounds.origin.x
                + group_width * point_index as f32
                + group_width * 0.14
                + bar_width * series_index as f32;
            window.paint_quad(fill(
                Bounds::new(
                    point(x, y.min(baseline)),
                    gpui::size(bar_width, (baseline - y).abs().max(px(1.0))),
                ),
                color,
            ));
        }
    }
}

fn paint_lines(
    bounds: Bounds<Pixels>,
    series: &[ChartSeries],
    fill_area: bool,
    window: &mut Window,
) {
    let (minimum_y, maximum_y) = y_range(series);
    let (minimum_x, maximum_x) = x_range(series);
    for (series_index, series) in series.iter().enumerate() {
        let points = series
            .points
            .iter()
            .map(|data| {
                point(
                    x_position(data.x, minimum_x, maximum_x, bounds),
                    y_position(data.y, minimum_y, maximum_y, bounds),
                )
            })
            .collect::<Vec<_>>();
        if points.is_empty() {
            continue;
        }
        let color = gpui::rgb(SERIES_COLORS[series_index % SERIES_COLORS.len()]);
        if fill_area {
            let mut area = PathBuilder::fill();
            area.move_to(point(points[0].x, bounds.origin.y + bounds.size.height));
            for point in &points {
                area.line_to(*point);
            }
            area.line_to(point(
                points.last().unwrap().x,
                bounds.origin.y + bounds.size.height,
            ));
            area.close();
            if let Ok(path) = area.build() {
                window.paint_path(path, color.alpha(0.18));
            }
        }
        let mut line = PathBuilder::stroke(px(2.0));
        line.move_to(points[0]);
        for point in points.iter().skip(1) {
            line.line_to(*point);
        }
        if let Ok(path) = line.build() {
            window.paint_path(path, color);
        }
    }
}

fn paint_scatter(bounds: Bounds<Pixels>, series: &[ChartSeries], window: &mut Window) {
    let (minimum_y, maximum_y) = y_range(series);
    let (minimum_x, maximum_x) = x_range(series);
    for (series_index, series) in series.iter().enumerate() {
        let color = gpui::rgb(SERIES_COLORS[series_index % SERIES_COLORS.len()]);
        for data in &series.points {
            let center = point(
                x_position(data.x, minimum_x, maximum_x, bounds),
                y_position(data.y, minimum_y, maximum_y, bounds),
            );
            window.paint_quad(gpui::quad(
                Bounds::new(
                    point(center.x - px(3.0), center.y - px(3.0)),
                    gpui::size(px(6.0), px(6.0)),
                ),
                px(3.0),
                color,
                px(0.0),
                gpui::transparent_black(),
                gpui::BorderStyle::Solid,
            ));
        }
    }
}

fn paint_pie(bounds: Bounds<Pixels>, series: &[ChartSeries], window: &mut Window) {
    let Some(series) = series.first() else {
        return;
    };
    let values = series
        .points
        .iter()
        .filter(|point| point.y > 0.0)
        .collect::<Vec<_>>();
    let total = values.iter().map(|point| point.y).sum::<f64>();
    if total <= 0.0 {
        return;
    }
    let radius = (bounds.size.width.min(bounds.size.height) * 0.36).max(px(20.0));
    let center = point(
        bounds.origin.x + bounds.size.width / 2.0,
        bounds.origin.y + bounds.size.height / 2.0,
    );
    let mut angle = -std::f32::consts::FRAC_PI_2;
    for (index, value) in values.iter().enumerate() {
        let sweep = (value.y / total) as f32 * std::f32::consts::TAU;
        let end_angle = angle + sweep;
        let start = point(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        let end = point(
            center.x + radius * end_angle.cos(),
            center.y + radius * end_angle.sin(),
        );
        let mut sector = PathBuilder::fill();
        sector.move_to(center);
        sector.line_to(start);
        if sweep >= std::f32::consts::TAU - 0.001 {
            let opposite = point(
                center.x - radius * angle.cos(),
                center.y - radius * angle.sin(),
            );
            sector.arc_to(point(radius, radius), px(0.0), false, true, opposite);
            sector.arc_to(point(radius, radius), px(0.0), false, true, end);
        } else {
            sector.arc_to(
                point(radius, radius),
                px(0.0),
                sweep > std::f32::consts::PI,
                true,
                end,
            );
        }
        sector.close();
        if let Ok(path) = sector.build() {
            window.paint_path(path, gpui::rgb(SERIES_COLORS[index % SERIES_COLORS.len()]));
        }
        angle = end_angle;
    }
}

fn inset_bounds(
    bounds: Bounds<Pixels>,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> Bounds<Pixels> {
    Bounds::new(
        point(bounds.origin.x + px(left), bounds.origin.y + px(top)),
        gpui::size(
            (bounds.size.width - px(left + right)).max(px(1.0)),
            (bounds.size.height - px(top + bottom)).max(px(1.0)),
        ),
    )
}

fn y_range(series: &[ChartSeries]) -> (f64, f64) {
    let mut minimum = 0.0_f64;
    let mut maximum = 0.0_f64;
    for value in series
        .iter()
        .flat_map(|series| series.points.iter().map(|point| point.y))
    {
        minimum = minimum.min(value);
        maximum = maximum.max(value);
    }
    if (maximum - minimum).abs() < f64::EPSILON {
        maximum = minimum + 1.0;
    }
    (minimum, maximum)
}

fn x_range(series: &[ChartSeries]) -> (f64, f64) {
    let mut values = series
        .iter()
        .flat_map(|series| series.points.iter().map(|point| point.x));
    let Some(first) = values.next() else {
        return (0.0, 1.0);
    };
    let (mut minimum, mut maximum) = (first, first);
    for value in values {
        minimum = minimum.min(value);
        maximum = maximum.max(value);
    }
    if (maximum - minimum).abs() < f64::EPSILON {
        maximum = minimum + 1.0;
    }
    (minimum, maximum)
}

fn x_position(value: f64, minimum: f64, maximum: f64, bounds: Bounds<Pixels>) -> Pixels {
    bounds.origin.x + bounds.size.width * ((value - minimum) / (maximum - minimum)) as f32
}

fn y_position(value: f64, minimum: f64, maximum: f64, bounds: Bounds<Pixels>) -> Pixels {
    bounds.origin.y + bounds.size.height * (1.0 - ((value - minimum) / (maximum - minimum)) as f32)
}

fn chart_type_id(chart_type: ChartType) -> &'static str {
    match chart_type {
        ChartType::Bar => "bar",
        ChartType::Line => "line",
        ChartType::Area => "area",
        ChartType::Scatter => "scatter",
        ChartType::Pie => "pie",
    }
}

fn chart_type_label(chart_type: ChartType, language: UiLanguage) -> &'static str {
    match chart_type {
        ChartType::Bar => text(language, "柱状", "Bar"),
        ChartType::Line => text(language, "折线", "Line"),
        ChartType::Area => text(language, "面积", "Area"),
        ChartType::Scatter => text(language, "散点", "Scatter"),
        ChartType::Pie => text(language, "饼图", "Pie"),
    }
}
