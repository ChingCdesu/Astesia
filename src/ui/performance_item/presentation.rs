use zed_ui::prelude::*;

use crate::{
    application::PerformanceSnapshot,
    platform::UiLanguage,
    ui::{engine_presentation::engine_label, localization::text},
};

use super::PerformanceItem;

struct MetricValue {
    label: String,
    value: String,
}

struct MetricSection {
    title: String,
    metrics: Vec<MetricValue>,
}

fn metric(label: impl Into<String>, value: impl Into<String>) -> MetricValue {
    MetricValue {
        label: label.into(),
        value: value.into(),
    }
}

pub(super) fn render_dashboard_content(
    snapshot: Option<&PerformanceSnapshot>,
    error: Option<&str>,
    loading: bool,
    db_type: crate::db::DbType,
    language: UiLanguage,
    cx: &mut Context<PerformanceItem>,
) -> AnyElement {
    match snapshot {
        Some(PerformanceSnapshot::Unavailable { engine }) => centered_state(
            text(
                language,
                "当前数据库不提供性能指标",
                "Performance metrics are unavailable for this database",
            ),
            Some(engine_label(*engine)),
        ),
        Some(snapshot) => render_snapshot(snapshot, language, cx),
        None if loading => centered_state(
            text(
                language,
                "正在加载性能指标…",
                "Loading performance metrics…",
            ),
            Some(engine_label(db_type)),
        ),
        None => centered_state(
            error.unwrap_or_else(|| text(language, "暂无性能指标", "No performance metrics yet")),
            Some(text(
                language,
                "检查数据库权限后重试",
                "Check database permissions and try again",
            )),
        ),
    }
}

pub(super) fn render_refresh_error(error: String, cx: &mut Context<PerformanceItem>) -> AnyElement {
    let status = cx.theme().status();
    h_flex()
        .flex_none()
        .gap_2()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(status.error_border)
        .bg(status.error_background)
        .child(
            Icon::new(IconName::Warning)
                .size(IconSize::XSmall)
                .color(Color::Custom(status.error)),
        )
        .child(
            Label::new(error)
                .size(LabelSize::XSmall)
                .color(Color::Custom(status.error))
                .line_clamp(2),
        )
        .into_any_element()
}

fn render_snapshot(
    snapshot: &PerformanceSnapshot,
    language: crate::platform::UiLanguage,
    cx: &mut Context<PerformanceItem>,
) -> AnyElement {
    let colors = cx.theme().colors().clone();
    v_flex()
        .id("performance-sections")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .p_3()
        .gap_3()
        .children(
            metric_sections(snapshot, language)
                .into_iter()
                .enumerate()
                .map(|(section_index, section)| {
                    v_flex()
                        .id(("performance-section", section_index))
                        .flex_none()
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.editor_background)
                        .child(
                            h_flex()
                                .h(px(30.0))
                                .flex_none()
                                .px_3()
                                .border_b_1()
                                .border_color(colors.border)
                                .bg(colors.panel_background)
                                .child(
                                    Label::new(section.title)
                                        .size(LabelSize::XSmall)
                                        .weight(gpui::FontWeight::SEMIBOLD),
                                ),
                        )
                        .child(h_flex().items_stretch().flex_wrap().children(
                            section.metrics.into_iter().enumerate().map(
                                |(metric_index, metric)| {
                                    v_flex()
                                        .id((
                                            "performance-metric",
                                            section_index * 100 + metric_index,
                                        ))
                                        .w(px(176.0))
                                        .min_h(px(62.0))
                                        .flex_none()
                                        .justify_center()
                                        .gap_1()
                                        .px_3()
                                        .py_2()
                                        .border_r_1()
                                        .border_b_1()
                                        .border_color(colors.border)
                                        .child(
                                            Label::new(metric.value)
                                                .size(LabelSize::Small)
                                                .weight(gpui::FontWeight::SEMIBOLD)
                                                .truncate(),
                                        )
                                        .child(
                                            Label::new(metric.label)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted)
                                                .truncate(),
                                        )
                                },
                            ),
                        ))
                }),
        )
        .into_any_element()
}

fn centered_state(message: impl Into<SharedString>, detail: Option<&str>) -> AnyElement {
    v_flex()
        .flex_1()
        .justify_center()
        .items_center()
        .gap_1()
        .p_6()
        .text_center()
        .child(Label::new(message.into()).size(LabelSize::Small))
        .when_some(detail, |element, detail| {
            element.child(
                Label::new(detail.to_string())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
        })
        .into_any_element()
}

fn metric_sections(snapshot: &PerformanceSnapshot, language: UiLanguage) -> Vec<MetricSection> {
    let section = |chinese, english, metrics| MetricSection {
        title: text(language, chinese, english).to_string(),
        metrics,
    };
    match snapshot {
        PerformanceSnapshot::MySql(data) => vec![
            section(
                "连接与吞吐",
                "Connections & Throughput",
                vec![
                    metric(
                        text(language, "当前连接", "Current Connections"),
                        data.threads_connected.to_string(),
                    ),
                    metric(
                        text(language, "累计连接", "Total Connections"),
                        format_count(data.connections),
                    ),
                    metric(
                        text(language, "活跃线程", "Active Threads"),
                        data.threads_running.to_string(),
                    ),
                    metric(
                        text(language, "查询总数", "Total Queries"),
                        format_count(data.queries),
                    ),
                    metric(
                        text(language, "慢查询", "Slow Queries"),
                        format_count(data.slow_queries),
                    ),
                    metric(
                        text(language, "缓存命中率", "Cache Hit Rate"),
                        format_percent(data.buffer_pool_hit_rate),
                    ),
                    metric(
                        text(language, "运行时间", "Uptime"),
                        format_uptime(data.uptime),
                    ),
                ],
            ),
            section(
                "网络与操作",
                "Network & Operations",
                vec![
                    metric(
                        text(language, "接收数据", "Bytes Received"),
                        format_bytes(data.bytes_received as f64),
                    ),
                    metric(
                        text(language, "发送数据", "Bytes Sent"),
                        format_bytes(data.bytes_sent as f64),
                    ),
                    metric("SELECT", format_count(data.selects)),
                    metric("INSERT", format_count(data.inserts)),
                    metric("UPDATE", format_count(data.updates)),
                    metric("DELETE", format_count(data.deletes)),
                ],
            ),
        ],
        PerformanceSnapshot::PostgreSql(data) => vec![
            section(
                "连接与事务",
                "Connections & Transactions",
                vec![
                    metric(
                        text(language, "活跃连接", "Active Connections"),
                        data.active_connections.to_string(),
                    ),
                    metric("Backends", data.backends.to_string()),
                    metric(
                        text(language, "提交事务", "Committed Transactions"),
                        format_signed_count(data.committed_transactions),
                    ),
                    metric(
                        text(language, "回滚事务", "Rolled-back Transactions"),
                        format_signed_count(data.rolled_back_transactions),
                    ),
                    metric(
                        text(language, "缓存命中率", "Cache Hit Rate"),
                        format_percent(data.cache_hit_ratio),
                    ),
                    metric(
                        text(language, "死锁", "Deadlocks"),
                        data.deadlocks.to_string(),
                    ),
                ],
            ),
            section(
                "元组与临时数据",
                "Tuples & Temporary Data",
                vec![
                    metric(
                        text(language, "返回元组", "Tuples Returned"),
                        format_signed_count(data.tuples_returned),
                    ),
                    metric(
                        text(language, "获取元组", "Tuples Fetched"),
                        format_signed_count(data.tuples_fetched),
                    ),
                    metric(
                        text(language, "插入元组", "Tuples Inserted"),
                        format_signed_count(data.tuples_inserted),
                    ),
                    metric(
                        text(language, "更新元组", "Tuples Updated"),
                        format_signed_count(data.tuples_updated),
                    ),
                    metric(
                        text(language, "删除元组", "Tuples Deleted"),
                        format_signed_count(data.tuples_deleted),
                    ),
                    metric(
                        text(language, "临时文件", "Temporary Files"),
                        format_signed_count(data.temporary_files),
                    ),
                    metric(
                        text(language, "临时数据", "Temporary Bytes"),
                        format_bytes(data.temporary_bytes.max(0) as f64),
                    ),
                    metric(
                        text(language, "磁盘读取块", "Blocks Read"),
                        format_signed_count(data.blocks_read),
                    ),
                    metric(
                        text(language, "缓存命中块", "Blocks Hit"),
                        format_signed_count(data.blocks_hit),
                    ),
                ],
            ),
        ],
        PerformanceSnapshot::SQLite(data) => vec![section(
            "数据库状态",
            "Database State",
            vec![
                metric(
                    text(language, "缓存大小", "Cache Size"),
                    data.cache_size.to_string(),
                ),
                metric(
                    text(language, "页面数", "Page Count"),
                    format_signed_count(data.page_count),
                ),
                metric(
                    text(language, "页面大小", "Page Size"),
                    format_bytes(data.page_size.max(0) as f64),
                ),
                metric(
                    text(language, "日志模式", "Journal Mode"),
                    data.journal_mode.clone(),
                ),
                metric("WAL Pages", format_signed_count(data.wal_pages)),
                metric(
                    text(language, "估算数据库大小", "Estimated Database Size"),
                    format_bytes(data.page_count.max(0) as f64 * data.page_size.max(0) as f64),
                ),
            ],
        )],
        PerformanceSnapshot::SqlServer(data) => vec![section(
            "服务器活动",
            "Server Activity",
            vec![
                metric(
                    text(language, "批处理请求/秒", "Batch Requests/sec"),
                    format_signed_count(data.batch_requests_per_second),
                ),
                metric(
                    text(language, "缓存命中率", "Buffer Cache Hit Rate"),
                    format_percent(data.buffer_cache_hit_ratio),
                ),
                metric(
                    text(language, "活跃会话", "Active Sessions"),
                    data.active_sessions.to_string(),
                ),
                metric(
                    text(language, "内存授权", "Memory Grants"),
                    data.memory_grants.to_string(),
                ),
                metric(
                    text(language, "页面预期寿命", "Page Life Expectancy"),
                    format!("{}s", data.page_life_expectancy.max(0)),
                ),
            ],
        )],
        PerformanceSnapshot::MongoDB(data) => vec![
            section(
                "连接与资源",
                "Connections & Resources",
                vec![
                    metric(
                        text(language, "连接数", "Connections"),
                        data.connections.to_string(),
                    ),
                    metric(
                        text(language, "常驻内存", "Resident Memory"),
                        format!("{} MB", format_decimal(data.resident_memory_mb)),
                    ),
                    metric(
                        text(language, "虚拟内存", "Virtual Memory"),
                        format!("{} MB", format_decimal(data.virtual_memory_mb)),
                    ),
                    metric(
                        text(language, "接收数据", "Bytes In"),
                        format_bytes(data.network_bytes_in as f64),
                    ),
                    metric(
                        text(language, "发送数据", "Bytes Out"),
                        format_bytes(data.network_bytes_out as f64),
                    ),
                    metric(
                        text(language, "运行时间", "Uptime"),
                        format_uptime(data.uptime_seconds),
                    ),
                ],
            ),
            section(
                "操作计数",
                "Operation Counters",
                vec![
                    metric("Insert", format_count(data.insert_operations)),
                    metric("Query", format_count(data.query_operations)),
                    metric("Update", format_count(data.update_operations)),
                    metric("Delete", format_count(data.delete_operations)),
                ],
            ),
        ],
        PerformanceSnapshot::Redis(data) => vec![
            section(
                "运行状态",
                "Runtime",
                vec![
                    metric(
                        text(language, "客户端连接", "Connected Clients"),
                        data.connected_clients.to_string(),
                    ),
                    metric(
                        text(language, "内存使用", "Memory Usage"),
                        fallback_value(&data.used_memory_human),
                    ),
                    metric(
                        text(language, "内存峰值", "Memory Peak"),
                        fallback_value(&data.used_memory_peak_human),
                    ),
                    metric(
                        text(language, "命令总数", "Total Commands"),
                        format_count(data.total_commands_processed),
                    ),
                    metric(
                        text(language, "运行时间", "Uptime"),
                        format_uptime(data.uptime_seconds),
                    ),
                    metric(
                        text(language, "版本", "Version"),
                        fallback_value(&data.version),
                    ),
                    metric(
                        text(language, "副本连接", "Connected Replicas"),
                        data.connected_replicas.to_string(),
                    ),
                ],
            ),
            section(
                "键空间",
                "Keyspace",
                vec![
                    metric(
                        text(language, "缓存命中", "Keyspace Hits"),
                        format_count(data.keyspace_hits),
                    ),
                    metric(
                        text(language, "缓存未命中", "Keyspace Misses"),
                        format_count(data.keyspace_misses),
                    ),
                    metric(
                        text(language, "命中率", "Hit Rate"),
                        format_percent(data.hit_rate),
                    ),
                    metric(
                        text(language, "淘汰键", "Evicted Keys"),
                        format_count(data.evicted_keys),
                    ),
                    metric(
                        text(language, "内存字节", "Memory Bytes"),
                        format_bytes(data.used_memory as f64),
                    ),
                    metric(
                        text(language, "峰值内存字节", "Peak Memory Bytes"),
                        format_bytes(data.used_memory_peak as f64),
                    ),
                ],
            ),
        ],
        PerformanceSnapshot::ClickHouse(data) => vec![
            section(
                "实时活动",
                "Live Activity",
                vec![
                    metric(
                        text(language, "活跃查询", "Active Queries"),
                        format_decimal(data.active_queries),
                    ),
                    metric(
                        text(language, "活跃合并", "Active Merges"),
                        format_decimal(data.active_merges),
                    ),
                    metric(
                        text(language, "活跃变更", "Active Mutations"),
                        format_decimal(data.active_mutations),
                    ),
                    metric(
                        text(language, "连接数", "Connections"),
                        format_decimal(data.connections),
                    ),
                    metric(
                        text(language, "内存使用", "Memory Usage"),
                        format_bytes(data.memory_usage),
                    ),
                    metric(
                        text(language, "运行时间", "Uptime"),
                        format_uptime(data.uptime.max(0.0) as u64),
                    ),
                ],
            ),
            section(
                "查询与数据",
                "Queries & Data",
                vec![
                    metric(
                        text(language, "查询总数", "Total Queries"),
                        format_decimal(data.total_queries),
                    ),
                    metric(
                        text(language, "失败查询", "Failed Queries"),
                        format_decimal(data.failed_queries),
                    ),
                    metric("SELECT", format_decimal(data.select_queries)),
                    metric("INSERT", format_decimal(data.insert_queries)),
                    metric(
                        text(language, "读取行数", "Selected Rows"),
                        format_decimal(data.selected_rows),
                    ),
                    metric(
                        text(language, "写入行数", "Inserted Rows"),
                        format_decimal(data.inserted_rows),
                    ),
                    metric(
                        text(language, "读取数据量", "Selected Bytes"),
                        format_bytes(data.selected_bytes),
                    ),
                    metric(
                        text(language, "写入数据量", "Inserted Bytes"),
                        format_bytes(data.inserted_bytes),
                    ),
                    metric(
                        text(language, "数据库数", "Databases"),
                        format_decimal(data.database_count),
                    ),
                    metric(
                        text(language, "表数量", "Tables"),
                        format_decimal(data.table_count),
                    ),
                ],
            ),
        ],
        PerformanceSnapshot::Unavailable { .. } => Vec::new(),
    }
}

fn format_count(value: u64) -> String {
    format_grouped_digits(&value.to_string())
}

fn format_signed_count(value: i64) -> String {
    let magnitude = value.unsigned_abs();
    let formatted = format_count(magnitude);
    if value < 0 {
        format!("-{formatted}")
    } else {
        formatted
    }
}

fn format_grouped_digits(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + value.len() / 3);
    for (index, character) in value.chars().enumerate() {
        if index > 0 && (value.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(character);
    }
    result
}

fn format_decimal(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    if value.fract().abs() < f64::EPSILON {
        return format_count(value.max(0.0) as u64);
    }
    format!("{:.2}", value.max(0.0))
}

fn format_percent(value: f64) -> String {
    format!("{}%", format_decimal(value))
}

fn format_bytes(value: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut scaled = value.max(0.0);
    let mut unit = 0;
    while scaled >= 1024.0 && unit < UNITS.len() - 1 {
        scaled /= 1024.0;
        unit += 1;
    }
    let display = if unit == 0 {
        format_decimal(scaled)
    } else {
        format!("{scaled:.1}")
    };
    format!("{display} {}", UNITS[unit])
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn fallback_value(value: &str) -> String {
    if value.is_empty() {
        "—".to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_dashboard_measurements_for_dense_display() {
        assert_eq!(format_count(12_345_678), "12,345,678");
        assert_eq!(format_signed_count(-12_345), "-12,345");
        assert_eq!(format_bytes(1_572_864.0), "1.5 MB");
        assert_eq!(format_uptime(90_061), "1d 1h 1m");
        assert_eq!(format_percent(99.95), "99.95%");
    }

    #[test]
    fn every_supported_engine_has_a_metric_presentation() {
        let snapshots = [
            PerformanceSnapshot::MySql(Default::default()),
            PerformanceSnapshot::PostgreSql(Default::default()),
            PerformanceSnapshot::SQLite(Default::default()),
            PerformanceSnapshot::SqlServer(Default::default()),
            PerformanceSnapshot::MongoDB(Default::default()),
            PerformanceSnapshot::Redis(Default::default()),
            PerformanceSnapshot::ClickHouse(Default::default()),
        ];

        for snapshot in snapshots {
            assert!(!metric_sections(&snapshot, crate::platform::UiLanguage::English).is_empty());
        }
    }
}
