//! Keeping a backend's reading history in step with the reader.
//!
//! The reader reports each page it shows ([`ReaderEvent::ShowingPage`]). The
//! reader launcher forwards those page indices here, and [`report_progress`]
//! turns each one into a [`ReadingProgress`] for the session's provider. Nothing
//! here is backend-specific: a provider that keeps reading history implements
//! [`Provider::update_reading_progress`], and the others ignore the call.
//!
//! [`ReaderEvent::ShowingPage`]: komasabi_reader::ReaderEvent::ShowingPage

use std::collections::HashMap;

use tokio::sync::mpsc::UnboundedReceiver;

use crate::providers::{PageRef, Provider, ReadingProgress};

/// Reports every page the reader shows to `provider`, until the reader's event
/// stream closes. `pages` carries indices into `plan`, the session's pages in
/// reading order.
///
/// Updates go out one at a time, in order. When pages arrive faster than the
/// backend answers (the user flicking through), the backlog is collapsed to the
/// last page of each run within one chapter (see [`collapse`]). Each chapter
/// still ends on its latest page, and a chapter finished on the way to the next
/// is still saved as finished. A failed update is logged and skipped. It
/// shouldn't interrupt reading, and the next page overwrites it anyway.
pub async fn report_progress(
    provider: &impl Provider,
    plan: &[PageRef],
    mut pages: UnboundedReceiver<usize>,
) {
    let chapter_pages = chapter_page_counts(plan);
    let mut last_reported = None;

    while let Some(first) = pages.recv().await {
        let mut batch = vec![first];
        while let Ok(index) = pages.try_recv() {
            batch.push(index);
        }

        for index in collapse(plan, &batch) {
            if last_reported == Some(index) {
                continue;
            }
            let Some(page) = plan.get(index) else {
                tracing::debug!(
                    "reader showed page {index}, beyond the {} planned",
                    plan.len()
                );
                continue;
            };

            let progress = ReadingProgress {
                page: page.clone(),
                chapter_pages: chapter_pages[page.chapter_id.as_str()],
            };
            match provider.update_reading_progress(&progress).await {
                Ok(()) => last_reported = Some(index),
                Err(error) => tracing::warn!("could not save reading progress: {error}"),
            }
        }
    }
}

/// How many pages each chapter in `plan` has, by [`PageRef::chapter_id`].
fn chapter_page_counts(plan: &[PageRef]) -> HashMap<&str, u16> {
    let mut counts = HashMap::new();
    for page in plan {
        *counts.entry(page.chapter_id.as_str()).or_insert(0u16) += 1;
    }
    counts
}

/// Drops the pages in `batch` that a later page in the same chapter supersedes.
/// Of each run of consecutive pages from one chapter, only the run's last page
/// is kept. Out-of-range indices are kept as they are, for the caller to skip.
fn collapse(plan: &[PageRef], batch: &[usize]) -> Vec<usize> {
    let chapter = |index: usize| plan.get(index).map(|page| page.chapter_id.as_str());

    batch
        .iter()
        .enumerate()
        .filter(|&(position, &index)| match batch.get(position + 1) {
            Some(&next) => chapter(index).is_none() || chapter(index) != chapter(next),
            None => true,
        })
        .map(|(_, &index)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tokio::sync::mpsc;

    use super::*;
    use crate::models::library_item::LibraryItem;
    use crate::models::series_entry::SeriesEntry;

    /// A provider that records the progress it's sent, failing the pages listed
    /// in `fail_pages`.
    #[derive(Default)]
    struct RecordingProvider {
        reported: Mutex<Vec<ReadingProgress>>,
        fail_pages: Vec<(String, u16)>,
    }

    impl RecordingProvider {
        fn reported(&self) -> Vec<(String, u16, u16)> {
            self.reported
                .lock()
                .unwrap()
                .iter()
                .map(|p| (p.page.chapter_id.clone(), p.page.page, p.chapter_pages))
                .collect()
        }
    }

    impl Provider for RecordingProvider {
        async fn list_items(
            &self,
            _: &str,
            _: Option<u32>,
            _: Option<u32>,
        ) -> Result<Vec<LibraryItem>, String> {
            unimplemented!()
        }
        async fn list_series_entries(&self, _: &str) -> Result<Vec<SeriesEntry>, String> {
            unimplemented!()
        }
        async fn cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            unimplemented!()
        }
        async fn volume_cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            unimplemented!()
        }
        async fn chapter_cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            unimplemented!()
        }
        async fn reader_pages(&self, _: &SeriesEntry) -> Result<Vec<PageRef>, String> {
            unimplemented!()
        }
        async fn reader_image(&self, _: &str, _: u16) -> Result<Vec<u8>, String> {
            unimplemented!()
        }

        async fn update_reading_progress(&self, progress: &ReadingProgress) -> Result<(), String> {
            let key = (progress.page.chapter_id.clone(), progress.page.page);
            if self.fail_pages.contains(&key) {
                return Err("server said no".to_string());
            }
            self.reported.lock().unwrap().push(progress.clone());
            Ok(())
        }
    }

    /// A volume of two chapters: "a" with 3 pages, then "b" with 2.
    fn volume_plan() -> Vec<PageRef> {
        [("a", 0), ("a", 1), ("a", 2), ("b", 0), ("b", 1)]
            .into_iter()
            .map(|(chapter_id, page)| PageRef {
                chapter_id: chapter_id.to_string(),
                page,
            })
            .collect()
    }

    /// Runs [`report_progress`] over `events`, all queued before it starts, so
    /// they arrive as a single backlog.
    fn report_backlog(provider: &RecordingProvider, events: &[usize]) {
        let (tx, rx) = mpsc::unbounded_channel();
        for &event in events {
            tx.send(event).unwrap();
        }
        drop(tx);
        run(report_progress(provider, &volume_plan(), rx));
    }

    /// Runs [`report_progress`] over `events`, letting it handle each one
    /// before the next arrives, the way a reader paged at a reading pace does.
    fn report_one_by_one(provider: &RecordingProvider, events: &[usize]) {
        let (tx, rx) = mpsc::unbounded_channel();
        let plan = volume_plan();
        run(async {
            let reader = async {
                for &event in events {
                    tx.send(event).unwrap();
                    tokio::task::yield_now().await;
                    tokio::task::yield_now().await;
                }
                drop(tx);
            };
            tokio::join!(report_progress(provider, &plan, rx), reader);
        });
    }

    fn run(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(test);
    }

    fn page(chapter_id: &str, page: u16, chapter_pages: u16) -> (String, u16, u16) {
        (chapter_id.to_string(), page, chapter_pages)
    }

    #[test]
    fn each_page_shown_is_reported_with_its_chapter_size() {
        let provider = RecordingProvider::default();

        report_one_by_one(&provider, &[0, 1, 3]);

        assert_eq!(
            provider.reported(),
            vec![page("a", 0, 3), page("a", 1, 3), page("b", 0, 2)]
        );
    }

    #[test]
    fn a_backlog_keeps_the_last_page_of_each_chapter_run() {
        let provider = RecordingProvider::default();

        report_backlog(&provider, &[0, 1, 2, 3, 4, 3]);

        // "a" ends on its last page (finished), then "b" ends where the user
        // turned back to.
        assert_eq!(provider.reported(), vec![page("a", 2, 3), page("b", 0, 2)]);
    }

    #[test]
    fn a_repeat_of_the_last_reported_page_is_skipped() {
        let provider = RecordingProvider::default();

        report_one_by_one(&provider, &[1, 1, 2]);

        assert_eq!(provider.reported(), vec![page("a", 1, 3), page("a", 2, 3)]);
    }

    #[test]
    fn a_failed_update_does_not_stop_reporting() {
        let provider = RecordingProvider {
            fail_pages: vec![("b".to_string(), 1)],
            ..Default::default()
        };

        report_one_by_one(&provider, &[4, 0]);

        assert_eq!(provider.reported(), vec![page("a", 0, 3)]);
    }

    #[test]
    fn pages_beyond_the_plan_are_ignored() {
        let provider = RecordingProvider::default();

        report_backlog(&provider, &[99]);

        assert!(provider.reported().is_empty());
    }

    #[test]
    fn finishes_chapter_is_true_only_on_the_last_page() {
        let progress = |page| ReadingProgress {
            page: PageRef {
                chapter_id: "a".to_string(),
                page,
            },
            chapter_pages: 3,
        };

        assert!(!progress(0).finishes_chapter());
        assert!(!progress(1).finishes_chapter());
        assert!(progress(2).finishes_chapter());
    }
}
