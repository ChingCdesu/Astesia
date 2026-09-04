use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QueryFileOperation {
    Open,
    Save,
}

#[derive(Clone, Debug)]
pub(crate) struct QueryFileRequest {
    generation: u64,
    operation: QueryFileOperation,
    path: PathBuf,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum QueryFileCompletion {
    Opened(String),
    Saved,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QueryFileError {
    pub code: &'static str,
    pub message: String,
}

impl QueryFileError {
    pub(crate) fn prompt(code: &'static str, error: impl std::fmt::Display) -> Self {
        Self {
            code,
            message: error.to_string(),
        }
    }

    fn io(operation: QueryFileOperation, path: &Path, error: impl std::fmt::Display) -> Self {
        let code = match operation {
            QueryFileOperation::Open => "query_file_open_failed",
            QueryFileOperation::Save => "query_file_save_failed",
        };
        Self {
            code,
            message: format!("{}: {error}", path.display()),
        }
    }

    pub(crate) fn task(error: impl std::fmt::Display) -> Self {
        Self {
            code: "query_file_task_failed",
            message: error.to_string(),
        }
    }
}

impl QueryFileRequest {
    pub(crate) async fn execute(&self) -> Result<QueryFileCompletion, QueryFileError> {
        match self.operation {
            QueryFileOperation::Open => tokio::fs::read_to_string(&self.path)
                .await
                .map(QueryFileCompletion::Opened)
                .map_err(|error| QueryFileError::io(self.operation, &self.path, error)),
            QueryFileOperation::Save => tokio::fs::write(&self.path, &self.text)
                .await
                .map(|_| QueryFileCompletion::Saved)
                .map_err(|error| QueryFileError::io(self.operation, &self.path, error)),
        }
    }
}

#[derive(Default)]
pub(crate) struct QueryFileState {
    path: Option<PathBuf>,
    current_text: String,
    saved_text: String,
    next_generation: u64,
    active_operation: Option<(u64, QueryFileOperation)>,
    error: Option<QueryFileError>,
}

impl QueryFileState {
    pub(crate) fn new(text: String) -> Self {
        Self {
            path: None,
            saved_text: text.clone(),
            current_text: text,
            next_generation: 0,
            active_operation: None,
            error: None,
        }
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn display_name(&self) -> Option<String> {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.current_text != self.saved_text
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.active_operation.is_some()
    }

    pub(crate) fn error(&self) -> Option<&QueryFileError> {
        self.error.as_ref()
    }

    pub(crate) fn update_text(&mut self, text: String) -> bool {
        if self.current_text == text {
            return false;
        }
        self.current_text = text;
        true
    }

    pub(crate) fn begin_open(&mut self, path: PathBuf) -> Option<QueryFileRequest> {
        self.begin(QueryFileOperation::Open, path, String::new())
    }

    pub(crate) fn begin_save(&mut self, path: PathBuf) -> Option<QueryFileRequest> {
        self.begin(QueryFileOperation::Save, path, self.current_text.clone())
    }

    fn begin(
        &mut self,
        operation: QueryFileOperation,
        path: PathBuf,
        text: String,
    ) -> Option<QueryFileRequest> {
        if self.active_operation.is_some() {
            return None;
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("query file generation exhausted");
        self.active_operation = Some((self.next_generation, operation));
        self.error = None;
        Some(QueryFileRequest {
            generation: self.next_generation,
            operation,
            path,
            text,
        })
    }

    pub(crate) fn finish(
        &mut self,
        request: &QueryFileRequest,
        result: Result<QueryFileCompletion, QueryFileError>,
    ) -> Option<QueryFileCompletion> {
        if self.active_operation != Some((request.generation, request.operation)) {
            return None;
        }
        self.active_operation = None;
        match result {
            Ok(QueryFileCompletion::Opened(text)) => {
                self.path = Some(request.path.clone());
                self.current_text.clone_from(&text);
                self.saved_text.clone_from(&text);
                self.error = None;
                Some(QueryFileCompletion::Opened(text))
            }
            Ok(QueryFileCompletion::Saved) => {
                self.path = Some(request.path.clone());
                self.saved_text.clone_from(&request.text);
                self.error = None;
                Some(QueryFileCompletion::Saved)
            }
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }

    pub(crate) fn set_error(&mut self, error: QueryFileError) {
        self.error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_tracks_file_identity_and_clean_contents() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("customer report.sql");
        tokio::fs::write(&path, "SELECT '二';\n")
            .await
            .expect("seed query file");
        let mut state = QueryFileState::new("SELECT 1;\n".to_string());

        let request = state.begin_open(path.clone()).expect("open request");
        let result = request.execute().await;
        assert_eq!(
            state.finish(&request, result),
            Some(QueryFileCompletion::Opened("SELECT '二';\n".to_string()))
        );

        assert_eq!(state.path(), Some(path.as_path()));
        assert_eq!(state.display_name().as_deref(), Some("customer report.sql"));
        assert!(!state.is_dirty());
        assert!(state.error().is_none());
    }

    #[tokio::test]
    async fn save_preserves_edits_made_while_the_write_was_in_flight() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("query.sql");
        let mut state = QueryFileState::new("SELECT 1;\n".to_string());
        state.update_text("SELECT 2;\n".to_string());

        let request = state.begin_save(path.clone()).expect("save request");
        state.update_text("SELECT 3;\n".to_string());
        let result = request.execute().await;
        assert_eq!(
            state.finish(&request, result),
            Some(QueryFileCompletion::Saved)
        );

        assert_eq!(
            tokio::fs::read_to_string(&path).await.expect("saved file"),
            "SELECT 2;\n"
        );
        assert!(state.is_dirty());
    }

    #[tokio::test]
    async fn successful_save_clears_dirty_state_and_sets_file_identity() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("query.sql");
        let mut state = QueryFileState::new("SELECT 1;\n".to_string());
        state.update_text("SELECT 2;\n".to_string());

        let request = state.begin_save(path.clone()).expect("save request");
        let result = request.execute().await;
        assert_eq!(
            state.finish(&request, result),
            Some(QueryFileCompletion::Saved)
        );

        assert_eq!(state.path(), Some(path.as_path()));
        assert!(!state.is_dirty());
        assert!(state.error().is_none());
    }

    #[tokio::test]
    async fn failed_operations_keep_identity_and_surface_a_stable_error_code() {
        let directory = tempfile::tempdir().expect("tempdir");
        let missing = directory.path().join("missing.sql");
        let mut state = QueryFileState::new("SELECT 1;\n".to_string());

        let request = state.begin_open(missing).expect("open request");
        let result = request.execute().await;
        assert_eq!(state.finish(&request, result), None);

        assert!(state.path().is_none());
        assert_eq!(state.error().unwrap().code, "query_file_open_failed");
        assert!(!state.is_busy());
    }
}
