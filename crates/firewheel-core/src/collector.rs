use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

/// A wrapper around `Arc` that automatically collects resources
/// from the audio thread and drops them on the main thread.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArcGc<T: ?Sized>(Arc<T>);

impl<T: Send + Sync + 'static> ArcGc<T> {
    pub fn new(value: T) -> Self {
        let value = Self(Arc::new(value));

        REGISTRY
            .lock()
            .unwrap()
            .push(Box::new(Arc::clone(&value.0)));

        value
    }
}

impl<T: ?Sized + Send + Sync + 'static> ArcGc<T> {
    pub fn new_unsized(f: impl FnOnce() -> Arc<T>) -> Self {
        let value = Self(f());

        REGISTRY
            .lock()
            .unwrap()
            .push(Box::new(Arc::clone(&value.0)));

        value
    }
}

impl<T: ?Sized + Send + Sync + 'static> Into<ArcGc<T>> for Arc<T> {
    fn into(self) -> ArcGc<T> {
        ArcGc::new_unsized(|| self)
    }
}

impl<T: ?Sized> core::ops::Deref for ArcGc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: ?Sized> Drop for ArcGc<T> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) == 2 {
            // Relaxed ordering should be sufficient since the collector can always
            // drop it on the next collect cycle.
            ANY_PTR_DROPPED.store(true, Ordering::Relaxed);
        }
    }
}

trait StrongCount: Send + Sync {
    fn count(&self) -> usize;
}

impl<T: Send + Sync + ?Sized> StrongCount for Arc<T> {
    fn count(&self) -> usize {
        Arc::strong_count(self)
    }
}

static REGISTRY: Mutex<Vec<Box<dyn StrongCount + 'static>>> = Mutex::new(Vec::new());
static ANY_PTR_DROPPED: AtomicBool = AtomicBool::new(false);

/// Collect and drop all unused [`ArcGc`] resources.
pub fn collect() {
    // Relaxed ordering should be sufficient since the collector can always
    // drop resources on the next collect cycle.
    if ANY_PTR_DROPPED.load(Ordering::Relaxed) {
        ANY_PTR_DROPPED.store(false, Ordering::Relaxed);

        let mut registry = REGISTRY.lock().unwrap();

        registry.retain(|ptr| ptr.count() > 1);
    }
}

impl<T: ?Sized> Clone for ArcGc<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn registry_size() -> usize {
        REGISTRY.lock().unwrap().len()
    }

    fn test_drop_works() {
        assert_eq!(registry_size(), 0);

        let value = ArcGc::new(1);

        assert_eq!(registry_size(), 1);
        assert_eq!(ANY_PTR_DROPPED.load(Ordering::Relaxed), false);

        collect();

        assert_eq!(registry_size(), 1);
        assert_eq!(ANY_PTR_DROPPED.load(Ordering::Relaxed), false);

        drop(value);

        // Even though we've dropped the "last reference,"
        // the inner drop won't be called until we do garbage
        // collection.
        assert_eq!(registry_size(), 1);
        assert_eq!(ANY_PTR_DROPPED.load(Ordering::Relaxed), true);

        collect();

        assert_eq!(registry_size(), 0);
        assert_eq!(ANY_PTR_DROPPED.load(Ordering::Relaxed), false);
    }

    fn test_unsized_works() {
        assert_eq!(registry_size(), 0);

        let value = ArcGc::new_unsized(|| Arc::<[i32]>::from([1, 2, 3]));

        assert_eq!(registry_size(), 1);
        assert_eq!(ANY_PTR_DROPPED.load(Ordering::Relaxed), false);

        collect();

        assert_eq!(registry_size(), 1);
        assert_eq!(ANY_PTR_DROPPED.load(Ordering::Relaxed), false);

        drop(value);

        assert_eq!(registry_size(), 1);
        assert_eq!(ANY_PTR_DROPPED.load(Ordering::Relaxed), true);

        collect();

        assert_eq!(registry_size(), 0);
        assert_eq!(ANY_PTR_DROPPED.load(Ordering::Relaxed), false);
    }

    // These have to be grouped into one test because
    // they all access a global context.
    //
    // This still isn't very robust -- no other tests
    // in this crate can use `ArcGc` types.
    #[test]
    fn test_shared() {
        test_drop_works();
        test_unsized_works();
    }
}
