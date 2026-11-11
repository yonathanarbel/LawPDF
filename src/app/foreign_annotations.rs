//! Revision-bound, read-only annotations from other PDF applications.
use super::*;
use crate::document_store::FileRevision;
use crate::model::ForeignAnnotation;

#[derive(Default)]
pub(super) struct ForeignAnnotationsUi {
    documents: HashMap<u64, ForeignCache>,
}

struct ForeignCache {
    revision: FileRevision,
    loading: bool,
    error: Option<String>,
    marks: Option<Arc<Vec<ForeignAnnotation>>>,
}

impl ForeignAnnotationsUi {
    pub(super) fn forget(&mut self, epoch: u64) {
        self.documents.remove(&epoch);
    }

    pub(super) fn worker_stopped(&mut self) {
        for cache in self.documents.values_mut().filter(|cache| cache.loading) {
            cache.loading = false;
            cache.error = Some("The document worker stopped while loading annotations.".to_owned());
        }
    }
}

impl PdfEditorApp {
    #[cfg(test)]
    pub(super) fn set_foreign_annotations_for_test(&mut self, marks: Vec<ForeignAnnotation>) {
        let revision = self.annotation_sessions[&self.document_epoch]
            .revision
            .clone();
        self.foreign_annotations_ui.documents.insert(
            self.document_epoch,
            ForeignCache {
                revision,
                loading: false,
                error: None,
                marks: Some(Arc::new(marks)),
            },
        );
        self.annotation_list_cache = None;
    }

    pub(super) fn ensure_foreign_annotations(&mut self) {
        let epoch = self.document_epoch;
        let Some(document) = &self.document else {
            return;
        };
        let Some(session) = self.annotation_sessions.get(&epoch) else {
            return;
        };
        let revision = session.revision.clone();
        if self
            .foreign_annotations_ui
            .documents
            .get(&epoch)
            .is_some_and(|cache| cache.revision == revision)
        {
            return;
        }
        let path = document.path.clone();
        self.foreign_annotations_ui.documents.insert(
            epoch,
            ForeignCache {
                revision: revision.clone(),
                loading: true,
                error: None,
                marks: None,
            },
        );
        if let Err(error) = self.render_tx.send(RenderRequest::ForeignAnnotations {
            document_epoch: epoch,
            path,
            expected_revision: revision,
        }) {
            if let Some(cache) = self.foreign_annotations_ui.documents.get_mut(&epoch) {
                cache.loading = false;
                cache.error = Some(error.to_string());
            }
        }
        self.annotation_list_cache = None;
    }

    pub(super) fn foreign_annotations(&self) -> Option<Arc<Vec<ForeignAnnotation>>> {
        let cache = self
            .foreign_annotations_ui
            .documents
            .get(&self.document_epoch)?;
        let session = self.annotation_sessions.get(&self.document_epoch)?;
        (cache.revision == session.revision)
            .then(|| cache.marks.clone())
            .flatten()
    }

    pub(super) fn foreign_annotations_loading(&self) -> bool {
        self.foreign_annotations_ui
            .documents
            .get(&self.document_epoch)
            .is_some_and(|cache| cache.loading)
    }

    pub(super) fn foreign_annotations_failed(&self) -> bool {
        self.foreign_annotations_ui
            .documents
            .get(&self.document_epoch)
            .is_some_and(|cache| cache.error.is_some())
    }

    pub(super) fn retry_foreign_annotations(&mut self) {
        self.foreign_annotations_ui.forget(self.document_epoch);
        self.ensure_foreign_annotations();
    }

    pub(super) fn accept_foreign_annotations(
        &mut self,
        epoch: u64,
        path: PathBuf,
        revision: FileRevision,
        result: Result<Vec<ForeignAnnotation>, String>,
    ) {
        let Some(session) = self.annotation_sessions.get(&epoch) else {
            return;
        };
        let Some(cache) = self.foreign_annotations_ui.documents.get_mut(&epoch) else {
            return;
        };
        if cache.revision != revision {
            return;
        }
        cache.loading = false;
        if session.revision != revision {
            self.foreign_annotations_ui.documents.remove(&epoch);
            return;
        }
        let correct_path = self
            .document
            .as_ref()
            .is_some_and(|document| self.document_epoch == epoch && document.path == path)
            || self
                .tabs
                .iter()
                .any(|tab| tab.document_epoch == epoch && tab.document.path == path);
        if !correct_path {
            cache.error =
                Some("The annotation result no longer matches the open document.".to_owned());
            return;
        }
        match result {
            Ok(marks) => {
                cache.marks = Some(Arc::new(marks));
                cache.error = None;
            }
            Err(error) => {
                cache.error = Some(error);
                cache.marks = None;
            }
        }
        self.annotation_list_cache = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_result_is_discarded_and_worker_failure_can_be_retried() {
        let (mut app, _ctx, requests) = annotation_saving::tests::fixture();
        let path = app.document.as_ref().unwrap().path.clone();
        let old_revision = app.annotation_sessions[&7].revision.clone();
        app.ensure_foreign_annotations();
        assert!(matches!(
            requests.try_recv().unwrap(),
            RenderRequest::ForeignAnnotations { .. }
        ));
        assert!(app.foreign_annotations_loading());
        app.annotation_sessions.get_mut(&7).unwrap().revision.sha256 = "new revision".into();
        app.accept_foreign_annotations(7, path, old_revision, Ok(Vec::new()));
        assert!(app.foreign_annotations().is_none());
        app.ensure_foreign_annotations();
        assert!(app.foreign_annotations_loading());
        app.foreign_annotations_ui.worker_stopped();
        assert!(!app.foreign_annotations_loading());
        assert!(app.foreign_annotations_failed());
        app.retry_foreign_annotations();
        assert!(app.foreign_annotations_loading());
    }
}
