use super::*;
use crate::ui::{
    document_item::DocumentItem, er_diagram_item::ErDiagramItem, mcp_service_item::McpServiceItem,
    object_definition_item::ObjectDefinitionKind, performance_item::PerformanceItem,
    redis_item::RedisItem, task_center_item::TaskCenterItem,
};

pub(super) trait WorkspaceItemBehavior {
    fn query(&self) -> Option<&Entity<QueryItem>> {
        None
    }

    fn has_unsaved_changes(&self, _cx: &App) -> bool {
        false
    }

    fn discard_name(&self, _language: UiLanguage, cx: &App) -> String {
        self.label("", cx)
    }

    fn label(&self, fallback: &str, cx: &App) -> String;

    fn element(&self) -> AnyElement;

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>);

    fn invalidate_target(&self, target: &QueryTarget, cx: &mut Context<AstesiaWorkspace>) {
        self.invalidate_session(&target.connection_id, target.session_generation, cx);
    }

    fn invalidate_session(
        &self,
        _connection_id: &str,
        _session_generation: u64,
        _cx: &mut Context<AstesiaWorkspace>,
    ) {
    }

    fn reconcile_sessions(
        &self,
        _snapshot: &ConnectionWorkspaceSnapshot,
        _cx: &mut Context<AstesiaWorkspace>,
    ) {
    }

    fn refresh_active_surface(&self, _cx: &mut Context<AstesiaWorkspace>) -> bool {
        false
    }
}

pub(super) struct WorkspaceItem(Box<dyn WorkspaceItemBehavior>);

impl WorkspaceItem {
    pub(super) fn new(item: impl WorkspaceItemBehavior + 'static) -> Self {
        Self(Box::new(item))
    }

    pub(super) fn query(&self) -> Option<&Entity<QueryItem>> {
        self.0.query()
    }

    pub(super) fn has_unsaved_changes(&self, cx: &App) -> bool {
        self.0.has_unsaved_changes(cx)
    }

    pub(super) fn discard_name(&self, language: UiLanguage, cx: &App) -> String {
        self.0.discard_name(language, cx)
    }

    pub(super) fn label(&self, fallback: &str, cx: &App) -> String {
        self.0.label(fallback, cx)
    }

    pub(super) fn element(&self) -> AnyElement {
        self.0.element()
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.0.focus(window, cx);
    }

    pub(super) fn invalidate_target(
        &self,
        target: &QueryTarget,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.0.invalidate_target(target, cx);
    }

    pub(super) fn invalidate_session(
        &self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.0
            .invalidate_session(connection_id, session_generation, cx);
    }

    pub(super) fn reconcile_sessions(
        &self,
        snapshot: &ConnectionWorkspaceSnapshot,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.0.reconcile_sessions(snapshot, cx);
    }

    pub(super) fn refresh_active_surface(&self, cx: &mut Context<AstesiaWorkspace>) -> bool {
        self.0.refresh_active_surface(cx)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WorkspaceItemKey {
    Query(WorkspaceTabId),
    TableStructure(QueryTarget, TableRef),
    DataGrid(QueryTarget, TableRef),
    ObjectDefinition {
        target: QueryTarget,
        kind: ObjectDefinitionKind,
        name: String,
    },
    Document(QueryTarget, TableRef),
    Redis(QueryTarget, String),
    TaskCenter,
    Performance(QueryTarget),
    ErDiagram(QueryTarget),
    McpService,
}

impl WorkspaceItemKey {
    pub(super) fn target(&self) -> Option<&QueryTarget> {
        match self {
            Self::TableStructure(target, _)
            | Self::DataGrid(target, _)
            | Self::Document(target, _)
            | Self::Redis(target, _)
            | Self::Performance(target)
            | Self::ErDiagram(target)
            | Self::ObjectDefinition { target, .. } => Some(target),
            Self::Query(_) | Self::TaskCenter | Self::McpService => None,
        }
    }
}

impl WorkspaceItemBehavior for Entity<QueryItem> {
    fn query(&self) -> Option<&Entity<QueryItem>> {
        Some(self)
    }

    fn has_unsaved_changes(&self, cx: &App) -> bool {
        self.read(cx).has_unsaved_changes()
    }

    fn discard_name(&self, language: UiLanguage, cx: &App) -> String {
        self.read(cx)
            .file_display_name()
            .unwrap_or_else(|| text(language, "未命名查询", "Untitled Query").to_string())
    }

    fn label(&self, fallback: &str, cx: &App) -> String {
        self.read(cx)
            .file_display_name()
            .unwrap_or_else(|| fallback.to_string())
    }

    fn element(&self) -> AnyElement {
        self.clone().into_any_element()
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.focus(window, cx));
    }

    fn invalidate_target(&self, target: &QueryTarget, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.invalidate_target(target, cx));
    }

    fn invalidate_session(
        &self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.update(cx, |item, cx| {
            item.invalidate_session(connection_id, session_generation, cx)
        });
    }

    fn reconcile_sessions(
        &self,
        snapshot: &ConnectionWorkspaceSnapshot,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.update(cx, |item, cx| item.reconcile_sessions(snapshot, cx));
    }

    fn refresh_active_surface(&self, cx: &mut Context<AstesiaWorkspace>) -> bool {
        self.update(cx, |item, cx| item.refresh_chart(cx))
    }
}

impl WorkspaceItemBehavior for Entity<TableStructureItem> {
    fn label(&self, _fallback: &str, cx: &App) -> String {
        self.read(cx).tab_label()
    }

    fn element(&self) -> AnyElement {
        self.clone().into_any_element()
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.focus(window, cx));
    }

    fn invalidate_session(
        &self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.update(cx, |item, cx| {
            item.invalidate_session(connection_id, session_generation, cx)
        });
    }
}

impl WorkspaceItemBehavior for Entity<ObjectDefinitionItem> {
    fn label(&self, _fallback: &str, cx: &App) -> String {
        self.read(cx).label()
    }

    fn element(&self) -> AnyElement {
        self.clone().into_any_element()
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.focus(window, cx));
    }

    fn invalidate_session(
        &self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.update(cx, |item, cx| {
            item.invalidate_session(connection_id, session_generation, cx)
        });
    }
}

impl WorkspaceItemBehavior for Entity<DataGridItem> {
    fn has_unsaved_changes(&self, cx: &App) -> bool {
        self.read(cx).has_unsaved_changes()
    }

    fn discard_name(&self, _language: UiLanguage, cx: &App) -> String {
        self.read(cx).table_name()
    }

    fn label(&self, _fallback: &str, cx: &App) -> String {
        self.read(cx).label()
    }

    fn element(&self) -> AnyElement {
        self.clone().into_any_element()
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.focus(window, cx));
    }

    fn invalidate_session(
        &self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.update(cx, |item, cx| {
            item.invalidate_session(connection_id, session_generation, cx)
        });
    }

    fn refresh_active_surface(&self, cx: &mut Context<AstesiaWorkspace>) -> bool {
        self.update(cx, |item, cx| item.refresh_active(cx));
        true
    }
}

impl WorkspaceItemBehavior for Entity<DocumentItem> {
    fn label(&self, _fallback: &str, cx: &App) -> String {
        self.read(cx).label()
    }

    fn element(&self) -> AnyElement {
        self.clone().into_any_element()
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.focus(window, cx));
    }

    fn invalidate_session(
        &self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.update(cx, |item, cx| {
            item.invalidate_session(connection_id, session_generation, cx)
        });
    }
}

impl WorkspaceItemBehavior for Entity<RedisItem> {
    fn label(&self, _fallback: &str, cx: &App) -> String {
        self.read(cx).label()
    }

    fn element(&self) -> AnyElement {
        self.clone().into_any_element()
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.focus(window, cx));
    }

    fn invalidate_session(
        &self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.update(cx, |item, cx| {
            item.invalidate_session(connection_id, session_generation, cx)
        });
    }
}

impl WorkspaceItemBehavior for Entity<TaskCenterItem> {
    fn label(&self, _fallback: &str, cx: &App) -> String {
        self.read(cx).label()
    }

    fn element(&self) -> AnyElement {
        self.clone().into_any_element()
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.focus(window, cx));
    }
}

impl WorkspaceItemBehavior for Entity<PerformanceItem> {
    fn label(&self, _fallback: &str, cx: &App) -> String {
        self.read(cx).label(cx)
    }

    fn element(&self) -> AnyElement {
        self.clone().into_any_element()
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.focus(window, cx));
    }

    fn invalidate_session(
        &self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.update(cx, |item, cx| {
            item.invalidate_session(connection_id, session_generation, cx)
        });
    }

    fn refresh_active_surface(&self, cx: &mut Context<AstesiaWorkspace>) -> bool {
        self.update(cx, |item, cx| item.refresh(cx));
        true
    }
}

impl WorkspaceItemBehavior for Entity<ErDiagramItem> {
    fn label(&self, _fallback: &str, cx: &App) -> String {
        self.read(cx).label(cx)
    }

    fn element(&self) -> AnyElement {
        self.clone().into_any_element()
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.focus(window, cx));
    }

    fn invalidate_session(
        &self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<AstesiaWorkspace>,
    ) {
        self.update(cx, |item, cx| {
            item.invalidate_session(connection_id, session_generation, cx)
        });
    }

    fn refresh_active_surface(&self, cx: &mut Context<AstesiaWorkspace>) -> bool {
        self.update(cx, |item, cx| item.refresh(cx));
        true
    }
}

impl WorkspaceItemBehavior for Entity<McpServiceItem> {
    fn label(&self, _fallback: &str, cx: &App) -> String {
        self.read(cx).label()
    }

    fn element(&self) -> AnyElement {
        self.clone().into_any_element()
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<AstesiaWorkspace>) {
        self.update(cx, |item, cx| item.focus(window, cx));
    }
}
