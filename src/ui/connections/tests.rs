use super::*;
use crate::application::{
    ConnectionProfileSnapshot, ConnectionWorkspaceSnapshot, DatabaseSessionSnapshot,
};
use crate::db::DbType;

#[gpui_kit::test]
fn expanded_sidebar_only_renders_visible_rows(cx: &mut gpui_kit::TestAppContext) {
    use crate::application::connection_workspace::{CatalogEntry, CatalogKind, CatalogSection};
    use crate::application::LoadedDatabases;
    let (window, _directory) = sidebar_test_window(cx);
    let panel = window.root(cx).unwrap();
    panel.update(cx, |panel, cx| {
        let request = panel.state.begin_refresh();
        panel
            .state
            .finish_refresh(request, Ok(snapshot(profile("primary"))));
        let request = panel.state.begin_database_load("primary").unwrap();
        panel.state.finish_database_load(
            &request,
            Ok(LoadedDatabases {
                session_generation: 7,
                databases: vec!["test".into()],
            }),
        );
        let target = QueryTarget {
            connection_id: "primary".into(),
            connection_name: "primary".into(),
            database: "test".into(),
            db_type: DbType::PostgreSQL,
            session_generation: 7,
        };
        let requests = panel.state.begin_object_load(&target).unwrap();
        for request in requests {
            let entry = match request.kind() {
                CatalogKind::Tables => CatalogEntry::Tables(CatalogSection::Ready(
                    (0..10_000)
                        .map(|index| TableInfo {
                            reference: TableRef::qualified("public", format!("table_{index:05}")),
                            row_count: None,
                            comment: None,
                        })
                        .collect(),
                )),
                CatalogKind::Schemas => {
                    CatalogEntry::Schemas(CatalogSection::Ready(vec!["public".into()]))
                }
                kind => CatalogEntry::failed(
                    kind,
                    "Test error with enough text to wrap over several lines in the sidebar.",
                ),
            };
            assert!(panel.state.finish_object_load(&request, entry));
        }
        panel
            .expanded_databases
            .insert(("primary".into(), 7, "test".into()));
        panel.sidebar_rendered_rows.borrow_mut().clear();
        panel.notify_sidebar(cx);
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        let rendered = panel.sidebar_rendered_rows.borrow();
        assert!(!rendered.is_empty(), "the real sidebar must render");
        assert!(
            rendered.len() < 200,
            "offscreen rows were rendered: {}",
            rendered.len()
        );
        assert!(panel.sidebar_row_keys.borrow().len() > 10_000);
    });
    let builds = panel.read_with(cx, |panel, _| panel.sidebar_model_builds.get());
    window
        .update(cx, |panel, window, cx| {
            panel.sidebar_rendered_rows.borrow_mut().clear();
            window.dispatch_event(
                gpui_kit::PlatformInput::ScrollWheel(gpui_kit::ScrollWheelEvent {
                    position: gpui_kit::point(px(100.0), px(150.0)),
                    delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.0), px(-240.0))),
                    modifiers: Default::default(),
                    touch_phase: gpui_kit::TouchPhase::Moved,
                }),
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert!(
            panel.sidebar_list.logical_scroll_top().item_ix > 0,
            "wheel must move the list"
        );
        assert_eq!(
            panel.sidebar_model_builds.get(),
            builds,
            "scroll must reuse the flattened model"
        );
        assert!(panel.sidebar_rendered_rows.borrow().len() < 200);
    });
    panel.update(cx, |panel, cx| {
        panel.sidebar_rendered_rows.borrow_mut().clear();
        panel.sidebar_list.scroll_to(gpui_kit::ListOffset {
            item_ix: 5000,
            offset_in_item: px(0.0),
        });
        cx.notify();
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        let rendered = panel.sidebar_rendered_rows.borrow();
        assert!(
            rendered.iter().any(|index| *index >= 5000),
            "scroll must render the destination"
        );
        assert!(
            rendered.len() < 200,
            "scroll rendered {} rows",
            rendered.len()
        );
    });
    panel.update(cx, |panel, cx| {
        panel
            .collapsed_schemas
            .insert(("primary".into(), 7, "test".into(), "public".into()));
        panel.notify_sidebar(cx);
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert!(
            panel.sidebar_row_keys.borrow().len() < 30,
            "collapsed descendants must leave the virtual list"
        );
        assert!(
            panel.sidebar_list.logical_scroll_top().item_ix < panel.sidebar_row_keys.borrow().len()
        );
    });
}

fn profile(id: &str) -> SharedConnectionProfile {
    SharedConnectionProfile {
        id: id.to_string(),
        name: id.to_string(),
        db_type: DbType::PostgreSQL,
        host: "127.0.0.1".to_string(),
        port: 5432,
        username: "tester".to_string(),
        database: None,
        color: None,
        group_name: None,
        tags: Vec::new(),
        has_credential: false,
        revision: 1,
        mcp_enabled: false,
    }
}

fn snapshot(profile: SharedConnectionProfile) -> ConnectionWorkspaceSnapshot {
    ConnectionWorkspaceSnapshot {
        repository_revision: 1,
        mcp_revision: 0,
        profiles: vec![ConnectionProfileSnapshot {
            profile,
            session: DatabaseSessionSnapshot {
                generation: Some(7),
            },
            mcp_usage: None,
        }],
    }
}

#[test]
fn replacing_profiles_clears_a_missing_selection() {
    let mut selected = Some("primary".to_string());
    let empty = ConnectionWorkspaceSnapshot {
        repository_revision: 2,
        mcp_revision: 0,
        profiles: Vec::new(),
    };

    reconcile_selected_profile(&mut selected, Some(&empty));

    assert!(selected.is_none());
}

#[test]
fn structured_status_tracks_selected_session_and_operation() {
    let mut state = ConnectionWorkspaceState::default();
    let refresh = state.begin_refresh();
    state.finish_refresh(refresh, Ok(snapshot(profile("primary"))));

    let status = derive_status(
        &state,
        Some("primary"),
        None,
        false,
        crate::platform::UiLanguage::Chinese,
    );
    assert_eq!(status.session, ConnectionSessionStatus::Connected);
    assert_eq!(status.activity, ConnectionActivityStatus::Ready);

    let status = derive_status(
        &state,
        Some("primary"),
        None,
        true,
        crate::platform::UiLanguage::Chinese,
    );
    assert_eq!(status.activity, ConnectionActivityStatus::Working);

    state.begin_operation("primary", ProfileOperationKind::Disconnecting);
    let status = derive_status(
        &state,
        Some("primary"),
        None,
        false,
        crate::platform::UiLanguage::Chinese,
    );
    assert_eq!(status.session, ConnectionSessionStatus::Disconnecting);
    assert_eq!(status.activity, ConnectionActivityStatus::Working);
}

#[test]
fn selecting_another_profile_does_not_replace_the_active_database_status() {
    let mut current = snapshot(profile("primary"));
    current.profiles.push(ConnectionProfileSnapshot {
        profile: profile("staging"),
        session: DatabaseSessionSnapshot { generation: None },
        mcp_usage: None,
    });
    let mut state = ConnectionWorkspaceState::default();
    let refresh = state.begin_refresh();
    state.finish_refresh(refresh, Ok(current));
    let target = QueryTarget {
        connection_id: "primary".into(),
        connection_name: "primary".into(),
        database: "analytics".into(),
        db_type: DbType::PostgreSQL,
        session_generation: 7,
    };
    let status = derive_status(
        &state,
        Some("staging"),
        Some(&target),
        false,
        crate::platform::UiLanguage::Chinese,
    );
    assert_eq!(status.summary, "primary / analytics");
    assert_eq!(status.session, ConnectionSessionStatus::Connected);
}

fn sidebar_test_window(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::WindowHandle<ConnectionProfilesPanel>,
    tempfile::TempDir,
) {
    use crate::connection_repository::SharedConnectionRepository;
    use crate::credential_vault::test_support::MemoryCredentialVault;
    use crate::platform::DesktopPreferences;

    cx.update(|cx| {
        crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let application = Arc::new(Application::with_repository(repository));
    let settings = cx.new(|_| ShellSettings::new(DesktopPreferences::default(), None));
    let window = cx.add_window(|window, cx| {
        ConnectionProfilesPanel::new_unloaded(application, settings, window, cx)
    });
    cx.run_until_parked();
    (window, directory)
}

#[gpui_kit::test]
fn first_catalog_load_preserves_parent_measurement(cx: &mut gpui_kit::TestAppContext) {
    use crate::application::connection_workspace::{CatalogEntry, CatalogKind, CatalogSection};
    use crate::application::LoadedDatabases;
    let (window, _directory) = sidebar_test_window(cx);
    let panel = window.root(cx).unwrap();
    let target = QueryTarget {
        connection_id: "primary".into(),
        connection_name: "primary".into(),
        database: "test".into(),
        db_type: DbType::PostgreSQL,
        session_generation: 7,
    };
    panel.update(cx, |panel, cx| {
        let request = panel.state.begin_refresh();
        panel
            .state
            .finish_refresh(request, Ok(snapshot(profile("primary"))));
        let request = panel.state.begin_database_load("primary").unwrap();
        panel.state.finish_database_load(
            &request,
            Ok(LoadedDatabases {
                session_generation: 7,
                databases: vec!["test".into()],
            }),
        );
        panel.notify_sidebar(cx);
    });
    cx.run_until_parked();
    let before = panel.read_with(cx, |panel, _| {
        panel.sidebar_list.bounds_for_item(2).unwrap()
    });
    let requests = panel.update(cx, |panel, cx| {
        let requests = panel.state.begin_object_load(&target).unwrap();
        panel
            .expanded_databases
            .insert(("primary".into(), 7, "test".into()));
        panel.notify_sidebar(cx);
        let snapshot = panel.state.snapshot().unwrap().clone();
        let _view = panel.render_virtual_profiles(&snapshot, cx);
        assert_eq!(
            panel.sidebar_list.bounds_for_item(2),
            Some(before),
            "first expansion must retain the existing database measurement"
        );
        requests
    });
    cx.run_until_parked();
    assert_eq!(
        panel.read_with(cx, |panel, _| panel.sidebar_list.bounds_for_item(2)),
        Some(before)
    );
    for request in requests {
        panel.update(cx, |panel, cx| {
            let result = match request.kind() {
                CatalogKind::Schemas => {
                    CatalogEntry::Schemas(CatalogSection::Ready(vec!["public".into()]))
                }
                CatalogKind::Tables => {
                    CatalogEntry::Tables(CatalogSection::Ready(vec![TableInfo {
                        reference: TableRef::qualified("public", "orders"),
                        row_count: None,
                        comment: None,
                    }]))
                }
                kind => CatalogEntry::failed(kind, "A catalog section could not be loaded."),
            };
            assert!(panel.state.finish_object_load(&request, result));
            panel.notify_sidebar(cx);
            let snapshot = panel.state.snapshot().unwrap().clone();
            let _view = panel.render_virtual_profiles(&snapshot, cx);
            assert_eq!(
                panel.sidebar_list.bounds_for_item(2),
                Some(before),
                "each async result must retain the existing database measurement"
            );
        });
        cx.run_until_parked();
        assert_eq!(
            panel.read_with(cx, |panel, _| panel.sidebar_list.bounds_for_item(2)),
            Some(before)
        );
    }
}

#[gpui_kit::test]
fn expanded_column_details_render_with_tooltips(cx: &mut gpui_kit::TestAppContext) {
    use super::catalog_tree::{CatalogDetail, CatalogTableKey};
    use crate::application::TableStructureSnapshot;
    use crate::db::ColumnInfo;
    let (window, _directory) = sidebar_test_window(cx);
    let panel = window.root(cx).unwrap();
    panel.update(cx, |panel, cx| {
        let target = QueryTarget {
            connection_id: "primary".into(),
            connection_name: "primary".into(),
            database: "test".into(),
            db_type: DbType::PostgreSQL,
            session_generation: 7,
        };
        let key = CatalogTableKey::new(&target, &TableRef::qualified("public", "orders"));
        panel.table_details.insert(
            key.clone(),
            CatalogDetail::Ready(TableStructureSnapshot {
                columns: vec![ColumnInfo {
                    name: "customer_display_name".into(),
                    data_type: "VARCHAR(128)".into(),
                    nullable: true,
                    is_primary_key: false,
                    default_value: None,
                    comment: None,
                }],
                indexes: vec![],
                constraints: None,
                foreign_keys: None,
            }),
        );
        let mut rows = Vec::new();
        panel.append_detail_rows(&key, target.db_type, 2, &mut rows);
        panel.sidebar_list.reset(rows.len());
        *panel.sidebar_rows_cache.borrow_mut() = Some(std::rc::Rc::new(rows));
        let request = panel.state.begin_refresh();
        panel
            .state
            .finish_refresh(request, Ok(snapshot(profile("primary"))));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(panel.read_with(cx, |panel, _| panel
        .sidebar_list
        .bounds_for_item(1)
        .is_some()));
}

#[gpui_kit::test]
fn double_click_keeps_the_single_click_sidebar_state(cx: &mut gpui_kit::TestAppContext) {
    use crate::application::connection_workspace::CatalogEntry;
    use crate::application::LoadedDatabases;
    let (window, _directory) = sidebar_test_window(cx);
    let panel = window.root(cx).unwrap();
    let target = QueryTarget {
        connection_id: "primary".into(),
        connection_name: "primary".into(),
        database: "test".into(),
        db_type: DbType::PostgreSQL,
        session_generation: 7,
    };
    panel.update(cx, |panel, cx| {
        let request = panel.state.begin_refresh();
        panel
            .state
            .finish_refresh(request, Ok(snapshot(profile("primary"))));
        let request = panel.state.begin_database_load("primary").unwrap();
        panel.state.finish_database_load(
            &request,
            Ok(LoadedDatabases {
                session_generation: 7,
                databases: vec!["test".into()],
            }),
        );
        for request in panel.state.begin_object_load(&target).unwrap() {
            panel.state.finish_object_load(
                &request,
                CatalogEntry::failed(request.kind(), "test catalog"),
            );
        }
        panel.notify_sidebar(cx);
    });
    cx.run_until_parked();
    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    let database_position = panel.read_with(&visual, |panel, _| {
        panel.sidebar_list.bounds_for_item(2).unwrap().center()
    });
    for click_count in [1, 2] {
        visual.simulate_event(gpui_kit::MouseDownEvent {
            position: database_position,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count,
            first_mouse: false,
        });
        visual.simulate_event(gpui_kit::MouseUpEvent {
            position: database_position,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count,
        });
        visual.run_until_parked();
        assert!(
            panel.read_with(&visual, |panel, _| panel.expanded_databases.contains(&(
                "primary".into(),
                7,
                "test".into()
            )))
        );
    }
    let group_position = panel.read_with(&visual, |panel, _| {
        panel.sidebar_list.bounds_for_item(0).unwrap().center()
    });
    for click_count in [1, 2] {
        visual.simulate_event(gpui_kit::MouseDownEvent {
            position: group_position,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count,
            first_mouse: false,
        });
        visual.simulate_event(gpui_kit::MouseUpEvent {
            position: group_position,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count,
        });
        visual.run_until_parked();
        assert!(panel.read_with(&visual, |panel, _| panel.collapsed_groups.contains(&None)));
    }

    panel.update(&mut visual, |panel, cx| {
        let mut disconnected = snapshot(profile("primary"));
        disconnected.profiles[0].session.generation = None;
        let request = panel.state.begin_refresh();
        panel.state.finish_refresh(request, Ok(disconnected));
        panel.collapsed_groups.clear();
        panel.notify_sidebar(cx);
    });
    visual.run_until_parked();
    let profile_position = panel.read_with(&visual, |panel, _| {
        panel.sidebar_list.bounds_for_item(1).unwrap().center()
    });
    for click_count in [1, 2] {
        visual.simulate_event(gpui_kit::MouseDownEvent {
            position: profile_position,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count,
            first_mouse: false,
        });
        visual.simulate_event(gpui_kit::MouseUpEvent {
            position: profile_position,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count,
        });
        assert!(panel.read_with(&visual, |panel, _| panel
            .state
            .operation("primary")
            .is_none()));
        visual.run_until_parked();
    }
}
