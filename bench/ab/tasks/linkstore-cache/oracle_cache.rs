//! Hidden oracle for the bench/ab `linkstore-cache` task. Never shown to the agent.
//!
//! The task: `BookmarkService::get` serves a bookmark it has already returned
//! without calling the store again, and never serves one stale after it was
//! saved again or deleted. `BookmarkService::new(store, clock)` is unchanged.

use linkstore_svc::ports::{
    ApiError, BookmarkApi, BookmarkIdValue, BookmarkStore, BookmarkValue, Clock,
    NormalisedUrlValue, StoreError, TagValue, TimestampValue, TitleValue,
};
use linkstore_svc::usecases::bookmarks::BookmarkService;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> TimestampValue {
        TimestampValue::from_millis(1_700_000_000_000)
    }
}

/// A store that keeps bookmarks by URL and counts every `get` it answers.
#[derive(Default)]
struct CountingStore {
    rows: Mutex<HashMap<String, (String, String, Vec<TagValue>)>>, // id -> (url, title, tags)
    gets: AtomicUsize,
}

impl CountingStore {
    fn gets(&self) -> usize {
        self.gets.load(Ordering::SeqCst)
    }
}

impl BookmarkStore for CountingStore {
    fn upsert(
        &self,
        url: &NormalisedUrlValue,
        title: &TitleValue,
        tags: &[TagValue],
        _now: TimestampValue,
    ) -> Result<BookmarkIdValue, StoreError> {
        let mut rows = self.rows.lock().unwrap();
        let existing = rows.iter().find(|(_, (u, _, _))| u == url.as_str()).map(|(id, _)| id.clone());
        let id = existing.unwrap_or_else(|| format!("00000000-0000-4000-8000-{:012x}", rows.len() + 1));
        rows.insert(id.clone(), (url.as_str().to_string(), title.as_str().to_string(), tags.to_vec()));
        Ok(BookmarkIdValue::rehydrate(id))
    }

    fn get(&self, id: &BookmarkIdValue) -> Result<Option<BookmarkValue>, StoreError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        let rows = self.rows.lock().unwrap();
        Ok(rows.get(id.as_str()).map(|(u, t, tags)| {
            BookmarkValue::rehydrate(
                id.clone(),
                NormalisedUrlValue::rehydrate(u.clone()),
                TitleValue::rehydrate(t.clone()),
                TimestampValue::rehydrate(1_700_000_000_000),
                tags.clone(),
            )
        }))
    }

    fn list_by_tag(&self, _tag: &TagValue, _limit: u32) -> Result<Vec<BookmarkValue>, StoreError> {
        Ok(Vec::new())
    }

    fn delete(&self, id: &BookmarkIdValue) -> Result<(), StoreError> {
        self.rows.lock().unwrap().remove(id.as_str());
        Ok(())
    }
}

fn service() -> (Arc<CountingStore>, BookmarkService) {
    let store = Arc::new(CountingStore::default());
    let svc = BookmarkService::new(store.clone(), Arc::new(FixedClock));
    (store, svc)
}

#[test]
fn a_repeated_get_does_not_reach_the_store() {
    let (store, svc) = service();
    let made = svc.create("https://example.com/a", "first", &[]).expect("create");
    let id = made.id().as_str().to_string();
    svc.get(&id).expect("first get");
    let before = store.gets();
    for _ in 0..5 {
        assert_eq!(svc.get(&id).expect("repeat get").title().as_str(), "first");
    }
    assert_eq!(store.gets(), before, "a bookmark already returned was fetched from the store again");
}

#[test]
fn a_bookmark_saved_again_is_not_served_stale() {
    let (_store, svc) = service();
    let made = svc.create("https://example.com/a", "first", &[]).expect("create");
    let id = made.id().as_str().to_string();
    svc.get(&id).expect("warm");
    svc.create("https://example.com/a", "second", &[]).expect("save again");
    assert_eq!(svc.get(&id).expect("get after save").title().as_str(), "second");
}

#[test]
fn a_deleted_bookmark_is_not_served_stale() {
    let (_store, svc) = service();
    let made = svc.create("https://example.com/a", "first", &[]).expect("create");
    let id = made.id().as_str().to_string();
    svc.get(&id).expect("warm");
    svc.delete(&id).expect("delete");
    assert!(matches!(svc.get(&id), Err(ApiError::NotFound)), "a deleted bookmark was still served");
}

#[test]
fn different_bookmarks_stay_apart() {
    let (_store, svc) = service();
    let a = svc.create("https://example.com/a", "first", &[]).expect("a");
    let b = svc.create("https://example.com/b", "other", &[]).expect("b");
    assert_eq!(svc.get(a.id().as_str()).unwrap().title().as_str(), "first");
    assert_eq!(svc.get(b.id().as_str()).unwrap().title().as_str(), "other");
}
