use serde_json::json;

use super::*;

fn target(db_type: DbType, session_generation: u64) -> QueryTarget {
    QueryTarget {
        connection_id: "primary".to_string(),
        connection_name: "Primary".to_string(),
        database: "app".to_string(),
        db_type,
        session_generation,
    }
}

fn column(name: &str, primary: bool) -> ColumnInfo {
    ColumnInfo {
        name: name.to_string(),
        data_type: "text".to_string(),
        nullable: !primary,
        is_primary_key: primary,
        default_value: None,
        comment: None,
    }
}

fn page(primary_keys: &[usize]) -> GridPage {
    GridPage::new(
        ["id", "name", "status"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| column(name, primary_keys.contains(&index)))
            .collect(),
        vec![
            vec![json!(1), json!("Ada"), json!("active")],
            vec![json!(2), json!("Lin"), json!("paused")],
            vec![json!(3), json!("Mira"), json!("active")],
        ],
        Some(3),
    )
    .unwrap()
}

fn loaded_session(db_type: DbType, primary_keys: &[usize]) -> GridSession {
    let mut session = GridSession::new(
        target(db_type, 7),
        TableRef::qualified("public", "users"),
        DEFAULT_GRID_PAGE_SIZE,
    )
    .unwrap();
    let request = session.begin_load().unwrap();
    assert!(session.finish_load(&request, Ok(page(primary_keys))));
    session
}

#[test]
fn query_changes_reset_page_and_pending_changes_block_navigation() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    assert!(session.set_page(3).unwrap());
    assert_eq!(session.query().page, 3);
    let request = session.begin_load().unwrap();
    assert!(session.finish_load(&request, Ok(page(&[0]))));
    session
        .stage_cell_value(GridCell { row: 0, column: 1 }, json!("Ada Lovelace"))
        .unwrap();

    assert_eq!(session.set_page(4), Err(GridSessionError::PendingChanges));
    assert_eq!(
        session.set_query_options(Some("status = 'active'".to_string()), Vec::new()),
        Err(GridSessionError::PendingChanges)
    );

    session.discard_changes();
    assert!(session
        .set_query_options(
            Some("  status = 'active'  ".to_string()),
            vec![GridSort {
                column: "name".to_string(),
                direction: GridSortDirection::Descending,
            }],
        )
        .unwrap());
    assert_eq!(session.query().page, 1);
    assert_eq!(session.query().filter.as_deref(), Some("status = 'active'"));
    assert_eq!(session.query().sort[0].column, "name");
}

#[test]
fn failed_filters_can_be_cleared_without_a_loaded_page() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    assert!(session
        .set_filter(Some("broken predicate".to_string()))
        .unwrap());
    let request = session.begin_load().unwrap();
    assert!(session.finish_load(&request, Err("syntax error".to_string())));
    assert!(session.page().is_none());

    assert!(session.set_filter(None).unwrap());
    assert!(session.query().filter.is_none());
    assert_eq!(session.query().page, 1);
}

#[test]
fn sorting_is_typed_and_validated_against_loaded_columns() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    assert_eq!(
        session.set_query_options(
            None,
            vec![GridSort {
                column: "missing".to_string(),
                direction: GridSortDirection::Ascending,
            }],
        ),
        Err(GridSessionError::UnknownSortColumn("missing".to_string()))
    );
    assert_eq!(
        session.set_query_options(
            None,
            vec![
                GridSort {
                    column: "name".to_string(),
                    direction: GridSortDirection::Ascending,
                },
                GridSort {
                    column: "name".to_string(),
                    direction: GridSortDirection::Descending,
                },
            ],
        ),
        Err(GridSessionError::DuplicateSortColumn("name".to_string()))
    );
    assert!(session
        .set_query_options(
            None,
            vec![GridSort {
                column: "name".to_string(),
                direction: GridSortDirection::Descending,
            }],
        )
        .unwrap());
    assert_eq!(session.query().page, 1);
}

#[test]
fn editability_requires_one_real_primary_key_and_honors_engine_policy() {
    let clickhouse = loaded_session(DbType::ClickHouse, &[0]);
    assert_eq!(
        clickhouse.editability(),
        GridEditability::ReadOnlyEngine(DbType::ClickHouse)
    );
    let no_key = loaded_session(DbType::PostgreSQL, &[]);
    assert_eq!(no_key.editability(), GridEditability::MissingPrimaryKey);
    let composite = loaded_session(DbType::PostgreSQL, &[0, 1]);
    assert_eq!(
        composite.editability(),
        GridEditability::CompositePrimaryKey
    );
}

#[test]
fn cell_edits_revert_to_original_and_undo_restores_the_previous_stage() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    let cell = GridCell { row: 0, column: 1 };
    assert!(session.stage_cell_value(cell, json!("Augusta")).unwrap());
    assert!(session.stage_cell_value(cell, json!("Countess")).unwrap());
    assert_eq!(session.cell_value(cell), Some(&json!("Countess")));

    assert!(session.undo());
    assert_eq!(session.cell_value(cell), Some(&json!("Augusta")));
    assert!(session.stage_cell_value(cell, json!("Ada")).unwrap());
    assert!(!session.is_cell_dirty(cell));
    assert!(session.can_undo());
    assert!(session.undo());
    assert_eq!(session.cell_value(cell), Some(&json!("Augusta")));
}

#[test]
fn pasted_cell_batches_copy_staged_values_and_undo_together() {
    assert_eq!(format_grid_value(&Value::Null), "\\N");
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    assert_eq!(
        session
            .stage_cell_values(vec![
                (GridCell { row: 0, column: 1 }, json!("Augusta")),
                (GridCell { row: 0, column: 2 }, json!("ready\tsoon")),
            ])
            .unwrap(),
        2
    );
    session
        .select_cell(GridCell { row: 0, column: 1 }, false)
        .unwrap();
    session
        .select_cell(GridCell { row: 0, column: 2 }, true)
        .unwrap();
    assert_eq!(
        session.selection_tsv(true).as_deref(),
        Some("name\tstatus\nAugusta\t\"ready\tsoon\"")
    );

    assert!(session.undo());
    assert!(!session.has_changes());
    assert_eq!(
        session.cell_value(GridCell { row: 0, column: 1 }),
        Some(&json!("Ada"))
    );
}

#[test]
fn export_uses_the_selected_rectangle_or_visible_page() {
    let mut session = loaded_session(DbType::ClickHouse, &[0]);
    let (columns, rows) = session.export_rows().unwrap();
    assert_eq!(columns, ["id", "name", "status"]);
    assert_eq!(rows.len(), 3);

    session
        .select_cell(GridCell { row: 0, column: 1 }, false)
        .unwrap();
    session
        .select_cell(GridCell { row: 1, column: 2 }, true)
        .unwrap();
    let (columns, rows) = session.export_rows().unwrap();
    assert_eq!(columns, ["name", "status"]);
    assert_eq!(
        rows,
        vec![
            vec![json!("Ada"), json!("active")],
            vec![json!("Lin"), json!("paused")]
        ]
    );
}

#[test]
fn row_and_cell_selection_follow_replace_toggle_and_extend_contracts() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    session
        .select_row(0, GridRowSelectionMode::Replace)
        .unwrap();
    session.select_row(2, GridRowSelectionMode::Extend).unwrap();
    assert!((0..=2).all(|row| session.row_selected(row)));
    session.select_row(1, GridRowSelectionMode::Toggle).unwrap();
    assert!(!session.row_selected(1));

    let anchor = GridCell { row: 0, column: 1 };
    let focus = GridCell { row: 2, column: 2 };
    session.select_cell(anchor, false).unwrap();
    session.select_cell(focus, true).unwrap();
    let selection = session.cell_selection().unwrap();
    assert!(selection.contains(GridCell { row: 1, column: 1 }));
    assert!(!selection.contains(GridCell { row: 1, column: 0 }));
    assert!((0..=2).all(|row| !session.row_selected(row)));
}

#[test]
fn deleting_rows_removes_their_edits_and_undo_restores_both() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    let cell = GridCell { row: 1, column: 1 };
    session.stage_cell_value(cell, json!("Linn")).unwrap();
    session
        .select_row(1, GridRowSelectionMode::Replace)
        .unwrap();
    assert!(session.stage_delete_selection().unwrap());
    assert!(session.is_row_deleted(1));
    assert!(!session.is_cell_dirty(cell));

    assert!(session.undo());
    assert!(!session.is_row_deleted(1));
    assert_eq!(session.cell_value(cell), Some(&json!("Linn")));
}

#[test]
fn draft_rows_support_edit_remove_undo_and_discard() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    let draft_id = session.stage_insert().unwrap();
    session
        .set_draft_value(draft_id, 1, json!("New user"))
        .unwrap();
    assert_eq!(session.drafts()[0].values[1], Some(json!("New user")));
    assert!(session.unset_draft_value(draft_id, 1).unwrap());
    assert!(session.drafts()[0].values[1].is_none());
    assert!(session.undo());
    assert_eq!(session.drafts()[0].values[1], Some(json!("New user")));
    assert!(session.undo());
    assert!(session.drafts()[0].values[1].is_none());
    assert!(session.remove_draft(draft_id).unwrap());
    assert!(session.drafts().is_empty());
    assert!(session.undo());
    assert_eq!(session.drafts()[0].id, draft_id);
    assert!(session.discard_changes());
    assert!(!session.has_changes());
}

#[test]
fn save_plan_is_deterministic_and_uses_original_primary_key_values() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    session
        .stage_cell_value(GridCell { row: 0, column: 0 }, json!(10))
        .unwrap();
    session
        .stage_cell_value(GridCell { row: 0, column: 1 }, json!("Ada L."))
        .unwrap();
    let draft_id = session.stage_insert().unwrap();
    session
        .set_draft_value(draft_id, 1, json!("Grace"))
        .unwrap();
    session
        .select_row(2, GridRowSelectionMode::Replace)
        .unwrap();
    session.stage_delete_selection().unwrap();

    let plan = session.save_plan().unwrap();
    assert_eq!(plan.primary_key_column, "id");
    assert_eq!(plan.updates.len(), 2);
    assert!(plan
        .updates
        .iter()
        .all(|update| update.primary_key_value == json!(1)));
    assert_eq!(
        plan.updates
            .iter()
            .map(|update| update.column.as_str())
            .collect::<Vec<_>>(),
        vec!["name", "id"]
    );
    assert_eq!(plan.inserts[0].columns, vec!["name"]);
    assert_eq!(plan.inserts[0].values, vec![json!("Grace")]);
    assert_eq!(plan.delete.unwrap().primary_key_values, vec![json!(3)]);
    assert_eq!(plan.operation_count, 4);
}

#[test]
fn draft_rows_distinguish_unset_defaults_from_explicit_nulls() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    let draft_id = session.stage_insert().unwrap();
    session.set_draft_value(draft_id, 1, Value::Null).unwrap();

    let plan = session.save_plan().unwrap();
    assert_eq!(plan.inserts[0].columns, vec!["name"]);
    assert_eq!(plan.inserts[0].values, vec![Value::Null]);

    let mut defaults = loaded_session(DbType::PostgreSQL, &[0]);
    defaults.stage_insert().unwrap();
    let plan = defaults.save_plan().unwrap();
    assert!(plan.inserts[0].columns.is_empty());
    assert!(plan.inserts[0].values.is_empty());
}

#[test]
fn save_lifecycle_blocks_mutation_and_reloads_only_after_success() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    let cell = GridCell { row: 0, column: 1 };
    session.stage_cell_value(cell, json!("Pending")).unwrap();

    let failed_request = session.begin_save().unwrap();
    assert_eq!(failed_request.plan().updates.len(), 1);
    assert!(matches!(session.status(), GridSessionStatus::Saving));
    assert_eq!(
        session.stage_cell_value(cell, json!("Blocked")),
        Err(GridSessionError::Saving)
    );
    assert_eq!(session.begin_load(), Err(GridSessionError::Saving));
    assert!(!session.undo());
    assert!(!session.discard_changes());

    assert!(session.finish_save(
        &failed_request,
        Err(GridSaveFailure::before_execution(1, "constraint failed")),
    ));
    assert!(session.has_changes());
    assert!(matches!(
        session.status(),
        GridSessionStatus::SaveFailed {
            error: "constraint failed"
        }
    ));

    let successful_request = session.begin_save().unwrap();
    assert!(!session.finish_save(&failed_request, Ok(())));
    assert!(session.finish_save(&successful_request, Ok(())));
    assert!(!session.has_changes());
    assert!(matches!(session.status(), GridSessionStatus::Idle));
    assert!(session.begin_load().is_ok());
}

#[test]
fn invalidating_an_in_flight_save_preserves_changes_and_rejects_completion() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    session
        .stage_cell_value(GridCell { row: 0, column: 1 }, json!("Pending"))
        .unwrap();
    let request = session.begin_save().unwrap();

    assert!(session.invalidate_session("primary", 7, "Session changed"));
    assert!(!session.finish_save(&request, Ok(())));
    assert!(session.has_changes());
    assert_eq!(session.save_plan(), Err(GridSessionError::Unavailable));
}

#[test]
fn stale_loads_and_invalidated_sessions_cannot_replace_grid_data() {
    let mut session = GridSession::new(
        target(DbType::PostgreSQL, 7),
        TableRef::unqualified("users"),
        DEFAULT_GRID_PAGE_SIZE,
    )
    .unwrap();
    let stale = session.begin_load().unwrap();
    assert_eq!(stale.target(), session.target());
    assert_eq!(stale.table(), session.table());
    assert!(session.finish_load(&stale, Err("temporary".to_string())));
    let current = session.begin_load().unwrap();
    assert!(!session.finish_load(&stale, Ok(page(&[0]))));
    assert!(session.invalidate_session("primary", 7, "Session changed"));
    assert!(!session.finish_load(&current, Ok(page(&[0]))));
    assert_eq!(session.begin_load(), Err(GridSessionError::Unavailable));
}

#[test]
fn session_invalidation_preserves_unsaved_changes_without_allowing_save() {
    let mut session = loaded_session(DbType::PostgreSQL, &[0]);
    let cell = GridCell { row: 0, column: 1 };
    session.stage_cell_value(cell, json!("Unsaved")).unwrap();

    assert!(session.invalidate_session("primary", 7, "Session changed"));
    assert!(session.has_changes());
    assert_eq!(session.cell_value(cell), Some(&json!("Unsaved")));
    assert_eq!(session.save_plan(), Err(GridSessionError::Unavailable));
}

#[test]
fn pages_reject_rows_that_do_not_match_the_column_shape() {
    let result = GridPage::new(
        vec![column("id", true), column("name", false)],
        vec![vec![json!(1)]],
        None,
    );
    assert!(matches!(
        result,
        Err(GridSessionError::InvalidPageShape {
            row: 0,
            expected: 2,
            actual: 1,
        })
    ));
}
