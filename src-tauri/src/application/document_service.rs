use serde_json::Value;

use crate::db::{DbType, DocumentPage, TableRef};

use super::{connections::ConnectionManager, QueryTarget};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DocumentQuery {
    pub(crate) page: u32,
    pub(crate) page_size: u32,
    pub(crate) filter: Option<Value>,
}

#[derive(Clone, Debug)]
pub(crate) struct DocumentLoadRequest {
    generation: u64,
    target: QueryTarget,
    collection: TableRef,
    query: DocumentQuery,
}

#[derive(Debug)]
enum DocumentState {
    Idle,
    Loading {
        generation: u64,
        page: Option<DocumentPage>,
    },
    Ready(DocumentPage),
    Failed {
        error: String,
        page: Option<DocumentPage>,
    },
    Unavailable {
        reason: String,
        page: Option<DocumentPage>,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DocumentSessionStatus<'a> {
    Idle,
    Loading,
    Ready,
    Failed(&'a str),
    Unavailable(&'a str),
}

#[derive(Debug)]
pub(crate) struct DocumentSession {
    target: QueryTarget,
    collection: TableRef,
    query: DocumentQuery,
    next_generation: u64,
    state: DocumentState,
}

impl DocumentSession {
    pub(crate) fn new(
        target: QueryTarget,
        collection: TableRef,
        page_size: u32,
    ) -> Result<Self, String> {
        if target.db_type != DbType::MongoDB {
            return Err("Document sessions require a MongoDB target".to_string());
        }
        if page_size == 0 {
            return Err("Document page size must be positive".to_string());
        }
        Ok(Self {
            target,
            collection,
            query: DocumentQuery {
                page: 1,
                page_size,
                filter: None,
            },
            next_generation: 0,
            state: DocumentState::Idle,
        })
    }

    pub(crate) fn target(&self) -> &QueryTarget {
        &self.target
    }

    pub(crate) fn collection(&self) -> &TableRef {
        &self.collection
    }

    pub(crate) fn query(&self) -> &DocumentQuery {
        &self.query
    }

    pub(crate) fn page(&self) -> Option<&DocumentPage> {
        match &self.state {
            DocumentState::Loading { page, .. }
            | DocumentState::Failed { page, .. }
            | DocumentState::Unavailable { page, .. } => page.as_ref(),
            DocumentState::Ready(page) => Some(page),
            DocumentState::Idle => None,
        }
    }

    pub(crate) fn status(&self) -> DocumentSessionStatus<'_> {
        match &self.state {
            DocumentState::Idle => DocumentSessionStatus::Idle,
            DocumentState::Loading { .. } => DocumentSessionStatus::Loading,
            DocumentState::Ready(_) => DocumentSessionStatus::Ready,
            DocumentState::Failed { error, .. } => DocumentSessionStatus::Failed(error),
            DocumentState::Unavailable { reason, .. } => DocumentSessionStatus::Unavailable(reason),
        }
    }

    pub(crate) fn begin_load(&mut self) -> Result<DocumentLoadRequest, String> {
        if matches!(self.state, DocumentState::Loading { .. }) {
            return Err("Documents are already loading".to_string());
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("document load generation exhausted");
        let generation = self.next_generation;
        let page = take_page(&mut self.state);
        self.state = DocumentState::Loading { generation, page };
        Ok(DocumentLoadRequest {
            generation,
            target: self.target.clone(),
            collection: self.collection.clone(),
            query: self.query.clone(),
        })
    }

    pub(crate) fn finish_load(
        &mut self,
        request: &DocumentLoadRequest,
        result: Result<DocumentPage, String>,
    ) -> bool {
        let state = std::mem::replace(&mut self.state, DocumentState::Idle);
        let DocumentState::Loading { generation, page } = state else {
            self.state = state;
            return false;
        };
        if generation != request.generation || request.query != self.query {
            self.state = DocumentState::Loading { generation, page };
            return false;
        }
        self.state = match result {
            Ok(page) => DocumentState::Ready(page),
            Err(error) => DocumentState::Failed { error, page },
        };
        true
    }

    pub(crate) fn set_filter(&mut self, filter_text: String) -> Result<bool, String> {
        self.require_not_loading()?;
        let filter_text = filter_text.trim().to_string();
        let filter = if filter_text.is_empty() {
            None
        } else {
            let value: Value = serde_json::from_str(&filter_text)
                .map_err(|error| format!("Invalid MongoDB JSON filter: {error}"))?;
            if !value.is_object() {
                return Err("MongoDB filter must be a JSON object".to_string());
            }
            Some(value)
        };
        if self.query.filter == filter {
            return Ok(false);
        }
        self.query.filter = filter;
        self.query.page = 1;
        Ok(true)
    }

    pub(crate) fn set_page(&mut self, page: u32) -> Result<bool, String> {
        self.require_not_loading()?;
        if page == 0 {
            return Err("Document page numbers start at 1".to_string());
        }
        if self.query.page == page {
            return Ok(false);
        }
        self.query.page = page;
        Ok(true)
    }

    pub(crate) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        reason: impl Into<String>,
    ) -> bool {
        if self.target.connection_id != connection_id
            || self.target.session_generation != session_generation
        {
            return false;
        }
        let page = take_page(&mut self.state);
        self.state = DocumentState::Unavailable {
            reason: reason.into(),
            page,
        };
        true
    }

    fn require_not_loading(&self) -> Result<(), String> {
        if matches!(self.state, DocumentState::Loading { .. }) {
            Err("Documents are loading".to_string())
        } else if matches!(self.state, DocumentState::Unavailable { .. }) {
            Err("Document session is unavailable".to_string())
        } else {
            Ok(())
        }
    }
}

fn take_page(state: &mut DocumentState) -> Option<DocumentPage> {
    let previous = std::mem::replace(state, DocumentState::Idle);
    match previous {
        DocumentState::Loading { page, .. }
        | DocumentState::Failed { page, .. }
        | DocumentState::Unavailable { page, .. } => page,
        DocumentState::Ready(page) => Some(page),
        DocumentState::Idle => None,
    }
}

#[derive(Clone)]
pub(crate) struct DocumentService {
    manager: ConnectionManager,
}

impl DocumentService {
    pub(super) fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub(crate) async fn load(&self, request: &DocumentLoadRequest) -> Result<DocumentPage, String> {
        let (handle, generation) = self
            .manager
            .driver_session(&request.target.connection_id)
            .await?;
        if generation != request.target.session_generation {
            return Err(format!(
                "MongoDB session changed before documents loaded (expected {}, found {generation})",
                request.target.session_generation
            ));
        }
        let driver = handle.lock_active().await?;
        if driver.db_type() != DbType::MongoDB {
            return Err("The selected Database Session is no longer MongoDB".to_string());
        }
        driver
            .get_documents(
                &request.target.database,
                &request.collection,
                request.query.filter.clone(),
                request.query.page,
                request.query.page_size,
            )
            .await
            .map_err(|error| format!("Could not load MongoDB documents: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> QueryTarget {
        QueryTarget {
            connection_id: "mongo".to_string(),
            connection_name: "Mongo".to_string(),
            database: "app".to_string(),
            db_type: DbType::MongoDB,
            session_generation: 7,
        }
    }

    #[test]
    fn filters_are_strict_json_objects_and_reset_paging() {
        let mut session = DocumentSession::new(target(), TableRef::unqualified("events"), 50)
            .expect("document session");
        session.set_page(3).unwrap();
        assert!(session.set_filter("{\"active\":true}".to_string()).unwrap());
        assert_eq!(session.query().page, 1);
        assert_eq!(
            session.query().filter,
            Some(serde_json::json!({"active": true}))
        );
        assert!(session.set_filter("[]".to_string()).is_err());
        assert!(session.set_filter("{broken".to_string()).is_err());
    }

    #[test]
    fn stale_loads_cannot_replace_newer_document_pages() {
        let mut session = DocumentSession::new(target(), TableRef::unqualified("events"), 50)
            .expect("document session");
        let stale = session.begin_load().unwrap();
        session.state = DocumentState::Loading {
            generation: stale.generation + 1,
            page: None,
        };
        assert!(!session.finish_load(
            &stale,
            Ok(DocumentPage {
                documents: vec![serde_json::json!({"_id": 1})],
                total_documents: 1,
            }),
        ));
    }
}
