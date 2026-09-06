use super::*;

fn column(name: &str, data_type: &str, nullable: bool, primary_key: bool) -> ColumnInfo {
    ColumnInfo {
        name: name.to_string(),
        data_type: data_type.to_string(),
        nullable,
        is_primary_key: primary_key,
        default_value: None,
        comment: None,
    }
}

fn parse_cell_input(
    column: &ColumnInfo,
    enum_values: &[String],
    input: &str,
    null_requested: bool,
) -> Result<Value, GridCellInputError> {
    GridColumn::new(column.clone(), enum_values.to_vec()).parse_input(input, null_requested)
}

#[test]
fn header_sort_cycles_ascending_descending_and_off() {
    let ascending = next_sort("name", &[]);
    assert_eq!(ascending[0].direction, GridSortDirection::Ascending);
    let descending = next_sort("name", &ascending);
    assert_eq!(descending[0].direction, GridSortDirection::Descending);
    assert!(next_sort("name", &descending).is_empty());
    assert_eq!(next_sort("id", &descending)[0].column, "id");
}

#[test]
fn pagination_prefers_total_rows_and_falls_back_to_full_pages() {
    let columns = Vec::new();
    let rows = vec![Vec::new(); 100];
    let known = GridPage::new(columns.clone(), rows.clone(), Some(100)).unwrap();
    assert!(!can_advance(&known, 1, 100));
    let unknown = GridPage::new(columns, rows, None).unwrap();
    assert!(can_advance(&unknown, 1, 100));
}

#[test]
fn keyboard_cursor_starts_at_origin_and_stays_inside_the_page() {
    let page = GridPage::new(
        vec![
            column("id", "integer", false, true),
            column("name", "text", false, false),
        ],
        vec![
            vec![serde_json::json!(1), serde_json::json!("Ada")],
            vec![serde_json::json!(2), serde_json::json!("Lin")],
        ],
        Some(2),
    )
    .unwrap();

    let origin = clamped_active_cell(Some(&page), None).unwrap();
    assert_eq!(origin, GridCell { row: 0, column: 0 });
    assert_eq!(
        moved_grid_cell(&page, origin, -1, -1),
        GridCell { row: 0, column: 0 }
    );
    assert_eq!(
        moved_grid_cell(&page, origin, 99, 99),
        GridCell { row: 1, column: 1 }
    );
}

#[test]
fn typed_cell_input_preserves_value_intent() {
    assert_eq!(
        parse_cell_input(&column("enabled", "boolean", false, false), &[], "1", false),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        parse_cell_input(
            &column("price", "decimal(10,2)", false, false),
            &[],
            "12.50",
            false,
        ),
        Ok(Value::String("12.50".to_string()))
    );
    assert_eq!(
        parse_cell_input(
            &column("payload", "jsonb", false, false),
            &[],
            "{\"ok\":true}",
            false,
        ),
        Ok(Value::String("{\"ok\":true}".to_string()))
    );
    assert_eq!(
        parse_cell_input(&column("note", "text", true, false), &[], "", false),
        Ok(Value::String(String::new()))
    );
    assert_eq!(
        parse_cell_input(&column("note", "text", true, false), &[], "ignored", true),
        Ok(Value::Null)
    );
    assert_eq!(
        parse_cell_input(
            &column("period", "interval", false, false),
            &[],
            "2 days",
            false,
        ),
        Ok(Value::String("2 days".to_string()))
    );
    assert_eq!(
        parse_cell_input(
            &column("day", "date", false, false),
            &[],
            "2026-09-02",
            false,
        ),
        Ok(Value::String("2026-09-02".to_string()))
    );
    assert_eq!(
        parse_cell_input(
            &column("started_at", "timestamp with time zone", false, false),
            &[],
            "2026-09-02T19:00:00+10:00",
            false,
        ),
        Ok(Value::String("2026-09-02T19:00:00+10:00".to_string()))
    );
    assert_eq!(
        parse_cell_input(
            &column("status", "mood", false, false),
            &["active".to_string(), "paused".to_string()],
            "active",
            false,
        ),
        Ok(Value::String("active".to_string()))
    );
    assert_eq!(
        parse_cell_input(&column("note", "text", false, false), &[], "NULL", false),
        Ok(Value::String("NULL".to_string()))
    );
    assert_eq!(
        parse_cell_input(&column("payload", "json", true, false), &[], "null", false),
        Ok(Value::String("null".to_string()))
    );
}

#[test]
fn typed_cell_input_rejects_invalid_or_forbidden_values() {
    assert_eq!(
        parse_cell_input(
            &column("count", "integer", false, false),
            &[],
            "many",
            false,
        ),
        Err(GridCellInputError::ExpectedInteger)
    );
    assert_eq!(
        parse_cell_input(&column("count", "integer", false, false), &[], "1.5", false,),
        Err(GridCellInputError::ExpectedInteger)
    );
    assert_eq!(
        parse_cell_input(&column("payload", "json", false, false), &[], "{", false),
        Err(GridCellInputError::InvalidJson)
    );
    assert_eq!(
        parse_cell_input(&column("name", "text", false, false), &[], "ignored", true,),
        Err(GridCellInputError::NullNotAllowed)
    );
    assert_eq!(
        parse_cell_input(&column("day", "date", false, false), &[], "tomorrow", false,),
        Err(GridCellInputError::ExpectedDate)
    );
    assert_eq!(
        parse_cell_input(
            &column("status", "account_status", false, false),
            &["active".to_string(), "paused".to_string()],
            "archived",
            false,
        ),
        Err(GridCellInputError::ExpectedEnum)
    );
    assert_eq!(
        parse_cell_input(
            &column("status", "enum", false, false),
            &[],
            "active",
            false
        ),
        Err(GridCellInputError::EnumValuesUnavailable)
    );
}

#[test]
fn editor_dirty_state_tracks_real_text_or_null_changes() {
    let initial = Value::String("NULL".to_string());
    assert!(!cell_editor_modified(Some(&initial), "NULL", "NULL", false));
    assert!(cell_editor_modified(Some(&initial), "NULL", "null", false));
    assert!(cell_editor_modified(Some(&initial), "NULL", "NULL", true));

    assert!(!cell_editor_modified(Some(&Value::Null), "", "", true));
    assert!(cell_editor_modified(Some(&Value::Null), "", "", false));
    assert!(!cell_editor_modified(None, "", "", false));
    assert!(cell_editor_modified(None, "", "Ada", false));
    assert!(cell_editor_modified(None, "", "", true));
}

#[test]
fn loaded_typed_values_commit_without_manufacturing_edits() {
    let columns = vec![
        column("id", "bigint", false, true),
        column("price", "numeric", false, false),
        column("object", "jsonb", false, false),
        column("scalar", "json", false, false),
        column("json_null", "jsonb", true, false),
        column("sql_null", "jsonb", true, false),
    ];
    let values = vec![
        serde_json::json!(1),
        Value::String("12.50".to_string()),
        Value::String("{\"ok\":true}".to_string()),
        Value::String("\"hello\"".to_string()),
        Value::String("null".to_string()),
        Value::Null,
    ];
    let mut session = GridSession::new(
        QueryTarget {
            connection_id: "primary".to_string(),
            connection_name: "Primary".to_string(),
            database: "app".to_string(),
            db_type: crate::db::DbType::PostgreSQL,
            session_generation: 1,
        },
        TableRef::qualified("public", "items"),
        DEFAULT_GRID_PAGE_SIZE,
    )
    .unwrap();
    let request = session.begin_load().unwrap();
    assert!(session.finish_load(
        &request,
        Ok(GridPage::new(columns.clone(), vec![values.clone()], Some(1)).unwrap())
    ));

    for column_index in 1..columns.len() {
        let original = &values[column_index];
        let parsed = parse_cell_input(
            &columns[column_index],
            &[],
            &edit_value(original),
            original.is_null(),
        )
        .unwrap();
        assert_eq!(parsed, *original);
        assert!(!session
            .stage_cell_value(
                GridCell {
                    row: 0,
                    column: column_index,
                },
                parsed,
            )
            .unwrap());
    }
    assert!(!session.has_changes());
}

#[test]
fn grid_paste_maps_headers_parses_types_and_rejects_overflow() {
    let page = GridPage::new(
        vec![
            column("id", "integer", false, true),
            column("name", "text", true, false),
            column("payload", "jsonb", true, false),
        ],
        vec![
            vec![serde_json::json!(1), serde_json::json!("Ada"), Value::Null],
            vec![serde_json::json!(2), serde_json::json!("Lin"), Value::Null],
        ],
        Some(2),
    )
    .unwrap();
    let assignments = grid_paste_assignments(
        &page,
        Some(GridCell { row: 0, column: 0 }),
        "name\tpayload\nAda Lovelace\t{\"ok\":true}\n\\N\tnull",
    )
    .unwrap();
    assert_eq!(
        assignments,
        vec![
            (
                GridCell { row: 0, column: 1 },
                serde_json::json!("Ada Lovelace")
            ),
            (
                GridCell { row: 0, column: 2 },
                Value::String("{\"ok\":true}".to_string())
            ),
            (GridCell { row: 1, column: 1 }, Value::Null),
            (
                GridCell { row: 1, column: 2 },
                Value::String("null".to_string())
            ),
        ]
    );
    assert_eq!(
        grid_paste_assignments(&page, Some(GridCell { row: 1, column: 2 }), "one,two",),
        Err(GridPasteError::OutOfBounds)
    );
}
