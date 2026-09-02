use super::*;

pub(super) enum WorkspaceItem {
    Query {
        item: Entity<QueryItem>,
        _document_subscription: Subscription,
    },
    TableStructure(Entity<TableStructureItem>),
    ObjectDefinition(Entity<ObjectDefinitionItem>),
    DataGrid {
        item: Entity<DataGridItem>,
        _observation: Subscription,
    },
}

impl WorkspaceItem {
    pub(super) fn query(&self) -> Option<&Entity<QueryItem>> {
        match self {
            Self::Query { item, .. } => Some(item),
            Self::TableStructure(_) | Self::ObjectDefinition(_) | Self::DataGrid { .. } => None,
        }
    }

    pub(super) fn has_unsaved_changes(&self, cx: &App) -> bool {
        match self {
            Self::Query { item, .. } => item.read(cx).has_unsaved_changes(),
            Self::DataGrid { item, .. } => item.read(cx).has_unsaved_changes(),
            Self::TableStructure(_) | Self::ObjectDefinition(_) => false,
        }
    }

    pub(super) fn discard_name(&self, language: UiLanguage, cx: &App) -> String {
        match self {
            Self::Query { item, .. } => item
                .read(cx)
                .file_display_name()
                .unwrap_or_else(|| text(language, "未命名查询", "Untitled Query").to_string()),
            Self::DataGrid { item, .. } => item.read(cx).table_name(),
            Self::TableStructure(item) => item.read(cx).label(),
            Self::ObjectDefinition(item) => item.read(cx).label(),
        }
    }

    pub(super) fn label(&self, fallback: &str, cx: &App) -> String {
        match self {
            Self::Query { item, .. } => item
                .read(cx)
                .file_display_name()
                .unwrap_or_else(|| fallback.to_string()),
            Self::TableStructure(item) => item.read(cx).label(),
            Self::ObjectDefinition(item) => item.read(cx).label(),
            Self::DataGrid { item, .. } => item.read(cx).label(cx),
        }
    }

    pub(super) fn element(&self) -> AnyElement {
        match self {
            Self::Query { item, .. } => item.clone().into_any_element(),
            Self::TableStructure(item) => item.clone().into_any_element(),
            Self::ObjectDefinition(item) => item.clone().into_any_element(),
            Self::DataGrid { item, .. } => item.clone().into_any_element(),
        }
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        match self {
            Self::Query { item, .. } => {
                item.update(cx, |item, cx| item.focus(window, cx));
            }
            Self::TableStructure(item) => {
                item.update(cx, |item, cx| item.focus(window, cx));
            }
            Self::ObjectDefinition(item) => {
                item.update(cx, |item, cx| item.focus(window, cx));
            }
            Self::DataGrid { item, .. } => {
                item.update(cx, |item, cx| item.focus(window, cx));
            }
        }
    }

    pub(super) fn matches_table_structure(
        &self,
        target: &QueryTarget,
        table: &TableRef,
        cx: &App,
    ) -> bool {
        matches!(
            self,
            Self::TableStructure(item) if item.read(cx).matches(target, table)
        )
    }

    pub(super) fn matches_data_grid(
        &self,
        target: &QueryTarget,
        table: &TableRef,
        cx: &App,
    ) -> bool {
        matches!(
            self,
            Self::DataGrid { item, .. } if item.read(cx).matches(target, table)
        )
    }

    pub(super) fn matches_object_definition(&self, object: &ObjectDefinition, cx: &App) -> bool {
        matches!(
            self,
            Self::ObjectDefinition(item) if item.read(cx).matches(object)
        )
    }

    pub(super) fn invalidate_target(
        &self,
        target: &QueryTarget,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        match self {
            Self::Query { item, .. } => {
                item.update(cx, |item, cx| item.invalidate_target(target, cx));
            }
            Self::TableStructure(item) => {
                item.update(cx, |item, cx| {
                    item.invalidate_session(&target.connection_id, target.session_generation, cx);
                });
            }
            Self::ObjectDefinition(item) => {
                item.update(cx, |item, cx| {
                    item.invalidate_session(&target.connection_id, target.session_generation, cx);
                });
            }
            Self::DataGrid { item, .. } => {
                item.update(cx, |item, cx| {
                    item.invalidate_session(&target.connection_id, target.session_generation, cx);
                });
            }
        }
    }

    pub(super) fn invalidate_session(
        &self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        match self {
            Self::Query { item, .. } => {
                item.update(cx, |item, cx| {
                    item.invalidate_session(connection_id, session_generation, cx)
                });
            }
            Self::TableStructure(item) => {
                item.update(cx, |item, cx| {
                    item.invalidate_session(connection_id, session_generation, cx)
                });
            }
            Self::ObjectDefinition(item) => {
                item.update(cx, |item, cx| {
                    item.invalidate_session(connection_id, session_generation, cx)
                });
            }
            Self::DataGrid { item, .. } => {
                item.update(cx, |item, cx| {
                    item.invalidate_session(connection_id, session_generation, cx)
                });
            }
        }
    }

    pub(super) fn reconcile_sessions(
        &self,
        snapshot: &ConnectionWorkspaceSnapshot,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        if let Self::Query { item, .. } = self {
            item.update(cx, |item, cx| item.reconcile_sessions(snapshot, cx));
        }
    }
}
