use super::*;

#[gpui_kit::test]
#[ignore = "render timing probe"]
fn measure_grid_scroll(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, _directory) = grid_test_window(cx);
    let mut medians = Vec::new();
    for (columns, value_len) in [(6, 20), (80, 20), (6, 20_000)] {
        view.update(cx, |item, cx| {
            let columns = (0..columns)
                .map(|ix| ColumnInfo {
                    name: format!("column_{ix}"),
                    data_type: "text".into(),
                    nullable: true,
                    is_primary_key: ix == 0,
                    default_value: None,
                    comment: None,
                })
                .collect::<Vec<_>>();
            let rows = (0..100)
                .map(|row| {
                    (0..columns.len())
                        .map(|col| Value::String(format!("{row}/{col} {}", "x".repeat(value_len))))
                        .collect()
                })
                .collect();
            let request = item.state.begin_load().unwrap();
            assert!(item.state.finish_load(
                &request,
                Ok(GridPage::new(columns, rows, Some(100)).unwrap())
            ));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let mut times = Vec::new();
        for row in 1..11 {
            let position = view.read_with(cx, |item, _| {
                item.horizontal_scroll_handle.bounds().center()
            });
            let start = std::time::Instant::now();
            cx.simulate_event(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(if row <= 5 { -40.0 } else { 40.0 }), px(0.0))),
                ..Default::default()
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            if row == 1 {
                assert!(
                    view.read_with(cx, |item, _| item.horizontal_scroll_handle.offset().x)
                        < px(0.0)
                );
            }
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "grid scroll: columns={columns}, value_len={value_len}, median={:.2}ms p95={:.2}ms",
            times[5], times[9]
        );
        medians.push(times[5]);
    }
    assert!(
        medians[1] < medians[0] * 2.0,
        "offscreen columns multiply scroll cost: {medians:?}"
    );
}

fn grid_test_window(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    Entity<DataGridItem>,
    &mut gpui_kit::VisualTestContext,
    tempfile::TempDir,
) {
    use crate::connection_repository::SharedConnectionRepository;
    use crate::credential_vault::test_support::MemoryCredentialVault;
    cx.update(|cx| {
        crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Dark, cx)
    });
    let directory = tempfile::tempdir().unwrap();
    let application = Arc::new(Application::with_repository(
        SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        ),
    ));
    let settings =
        cx.new(|_| ShellSettings::new(crate::platform::DesktopPreferences::default(), None));
    let (view, cx) = cx.add_window_view(|window, cx| {
        DataGridItem::new_unloaded(
            application,
            QueryTarget {
                connection_id: "fixture".into(),
                connection_name: "Fixture".into(),
                database: "fixture".into(),
                db_type: crate::db::DbType::PostgreSQL,
                session_generation: 1,
            },
            TableRef::qualified("public", "fixture"),
            settings,
            window,
            cx,
        )
    });
    (view, cx, directory)
}

#[gpui_kit::test]
fn virtual_grid_scrolls_to_distant_rows_without_rendering_the_page(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, _directory) = grid_test_window(cx);
    view.update(cx, |item, cx| {
        let columns = vec![ColumnInfo {
            name: "id".into(),
            data_type: "int4".into(),
            nullable: false,
            is_primary_key: true,
            default_value: None,
            comment: None,
        }];
        let rows = (0..10_000).map(|row| vec![Value::from(row)]).collect();
        let request = item.state.begin_load().unwrap();
        assert!(item.state.finish_load(
            &request,
            Ok(GridPage::new(columns, rows, Some(10_000)).unwrap())
        ));
        item.state
            .select_cell(GridCell { row: 0, column: 0 }, false)
            .unwrap();
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let cell_bounds = cx.debug_bounds("grid-cell-0-0").unwrap();
    for click_count in [1, 2] {
        cx.simulate_event(MouseDownEvent {
            position: cell_bounds.center(),
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position: cell_bounds.center(),
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count,
        });
    }
    cx.update(|window, cx| window.draw(cx).clear(cx));
    view.read_with(cx, |item, cx| {
        let editor = item
            .editing
            .as_ref()
            .expect("double-click starts inline editing");
        let input = editor.editor.read(cx).input_bounds(cx);
        assert!(input.top() >= cell_bounds.top() && input.bottom() <= cell_bounds.bottom());
    });
    assert_eq!(
        cx.debug_bounds("grid-cell-0-0").unwrap().size.height,
        cell_bounds.size.height
    );
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |item, _| assert!(item.editing.is_none()));
    cx.simulate_event(MouseDownEvent {
        position: cell_bounds.center(),
        button: MouseButton::Right,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    });
    view.read_with(cx, |item, _| assert!(item.context_menu.is_some()));
    cx.simulate_keystrokes("down enter");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("0")
        )
    });
    view.update(cx, |item, cx| {
        assert!(
            item.rendered_rows
                .borrow()
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                < 200
        );
        item.rendered_rows.borrow_mut().clear();
        item.rows_scroll_handle
            .scroll_to_item(9999, ScrollStrategy::Bottom);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    view.read_with(cx, |item, _| {
        let rendered = item.rendered_rows.borrow();
        assert!(rendered.contains(&9999));
        assert!(
            rendered
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                < 200
        );
        assert!(item
            .state
            .cell_selection()
            .unwrap()
            .contains(GridCell { row: 0, column: 0 }));
    });
    let draft = view.update(cx, |item, cx| {
        let draft = item.state.stage_insert().unwrap();
        item.rows_scroll_handle
            .scroll_to_item(10_000, ScrollStrategy::Bottom);
        cx.notify();
        draft
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    view.update(cx, |item, cx| {
        assert_eq!(
            item.state.page().unwrap().rows.len() + item.state.drafts().len(),
            10_001
        );
        assert!(item.rendered_rows.borrow().contains(&10_000));
        item.state.remove_draft(draft).unwrap();
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    view.read_with(cx, |item, _| {
        assert_eq!(
            item.state.page().unwrap().rows.len() + item.state.drafts().len(),
            10_000
        )
    });
    view.update(cx, |item, cx| {
        let columns = (0..80)
            .map(|index| ColumnInfo {
                name: format!("column_{index}"),
                data_type: "text".into(),
                nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
            })
            .collect();
        let request = item.state.begin_load().unwrap();
        assert!(item.state.finish_load(
            &request,
            Ok(GridPage::new(columns, Vec::new(), Some(0)).unwrap())
        ));
        item.rows_scroll_handle
            .scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let empty = cx.debug_bounds("grid-empty").unwrap();
    view.update(cx, |item, cx| {
        item.horizontal_scroll_handle
            .set_offset(point(px(-800.0), px(0.0)));
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(cx.debug_bounds("grid-empty").unwrap(), empty);
}
