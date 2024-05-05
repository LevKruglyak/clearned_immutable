use super::{ObjectStore, StoreID};
use core::panic;
use id_allocator::IDAllocator;
use serde::{Deserialize, Serialize};
use std::{
    cell::{Ref, RefCell, RefMut},
    collections::{HashMap, HashSet},
    path::Path,
    rc::Rc,
};

#[derive(Serialize, Deserialize, Clone)]
struct GlobalStoreCatalog {
    ids: IDAllocator<StoreID>,
    registry: HashMap<String, StoreID>,
}

const CACHE_SIZE: usize = 10_000;

const GLOBAL_STORE_CATALOG_ID: StoreID = 0;

impl Default for GlobalStoreCatalog {
    fn default() -> Self {
        let mut ids = IDAllocator::default();
        // Reserve the zero page for the global catalog
        // TODO: native way to do this in IDAllocator
        assert_eq!(ids.allocate(), GLOBAL_STORE_CATALOG_ID);

        Self {
            ids,
            registry: Default::default(),
        }
    }
}

pub struct GlobalStore {
    inner: Rc<RefCell<GlobalStoreInner>>,
}

struct GlobalStoreInner {
    store: marble::Marble,
    cache: HashMap<StoreID, Vec<u8>>,

    active_stores: HashSet<String>,
    catalog: GlobalStoreCatalog,
}

impl GlobalStoreInner {
    pub fn flush_cache(&mut self) -> crate::Result<()> {
        let mut batch = Vec::new();
        {
            for (&id, data) in self.cache.iter() {
                batch.push((id, Some(data.clone())));
            }
        }

        self.store.write_batch(batch)?;
        self.cache.clear();

        Ok(())
    }
}

impl GlobalStore {
    pub fn load(path: impl AsRef<Path>) -> crate::Result<Self> {
        let store = marble::open(path.as_ref())?;

        // Load catalog
        let catalog = match store.read(GLOBAL_STORE_CATALOG_ID)? {
            Some(data) => bincode::deserialize(&data)?,
            None => {
                let catalog = GlobalStoreCatalog::default();
                let data = bincode::serialize(&catalog).unwrap();

                store.write_batch([(GLOBAL_STORE_CATALOG_ID, Some(&data))])?;
                catalog
            }
        };

        Ok(GlobalStore {
            inner: Rc::new(RefCell::new(GlobalStoreInner {
                store,
                cache: HashMap::new(),
                catalog,
                active_stores: HashSet::new(),
            })),
        })
    }

    pub fn load_local_store<C>(&mut self, ident: impl ToString) -> crate::Result<LocalStore<C>>
    where
        C: for<'de> Deserialize<'de> + Serialize + Default + Clone,
    {
        if self.inner_ref().active_stores.contains(&ident.to_string()) {
            panic!("Catalog `{}` has already been loaded!", ident.to_string());
        }

        let registry = self
            .inner_ref()
            .catalog
            .registry
            .get(&ident.to_string())
            .copied();

        let id = registry.unwrap_or_else(|| {
            let id = self.allocate_page();
            self.inner_ref_mut()
                .catalog
                .registry
                .insert(ident.to_string(), id);
            id
        });

        let catalog = match self.read_page::<C>(id)? {
            Some(catalog) => catalog,
            None => {
                let catalog = C::default();
                self.write_page(&catalog, id)?;
                catalog
            }
        };

        self.inner_ref_mut().active_stores.insert(ident.to_string());

        Ok(LocalStore {
            root: self.inner.clone(),
            catalog,
            id,
            ident: ident.to_string(),
        })
    }

    pub fn flush(&mut self) -> crate::Result<()> {
        let catalog = self.inner_ref_mut().catalog.clone();
        self.write_page(&catalog, GLOBAL_STORE_CATALOG_ID)?;
        self.inner_ref_mut().flush_cache()?;

        Ok(())
    }

    pub fn stats(&self) -> marble::Stats {
        self.inner_ref().store.stats()
    }
}

impl Drop for GlobalStore {
    fn drop(&mut self) {
        assert_eq!(
            Rc::strong_count(&self.inner),
            1,
            "Shutting down global object store, but not all local object stores have been freed!"
        );

        self.flush().expect("Failed to flush GlobalStore to disk!");

        self.inner_ref_mut()
            .store
            .maintenance()
            .expect("Defragmentation failed!");
    }
}

pub struct LocalStore<C>
where
    C: Clone + Serialize,
{
    root: Rc<RefCell<GlobalStoreInner>>,
    pub catalog: C,
    id: StoreID,
    ident: String,
}

impl<C> LocalStore<C>
where
    C: Clone + Serialize,
{
    pub fn flush(&mut self) -> crate::Result<()> {
        let catalog = self.catalog.clone();
        self.write_page(&catalog, self.id)
    }
}

impl<C> Drop for LocalStore<C>
where
    C: Clone + Serialize,
{
    fn drop(&mut self) {
        self.inner_ref_mut().active_stores.remove(&self.ident);
        self.flush().expect("Failed to flush GlobalStore to disk!");
    }
}

impl<T> ObjectStore for T
where
    T: ObjectStoreInner,
{
    fn allocate_page(&mut self) -> StoreID {
        self.inner_ref_mut().catalog.ids.allocate()
    }

    fn free_page(&mut self, id: StoreID) -> crate::Result<bool> {
        if self.inner_ref_mut().catalog.ids.free(id) {
            let empty_page: Option<[u8; 1]> = None;
            self.inner_ref_mut().store.write_batch([(id, empty_page)])?;
            self.inner_ref_mut().cache.remove(&id);
            return Ok(true);
        }

        Ok(false)
    }

    fn clear(&mut self) -> crate::Result<()> {
        let mut clear_batch: Vec<(StoreID, Option<[u8; 1]>)> = vec![];

        for id in self.inner_ref().catalog.ids.iter() {
            clear_batch.push((id, None));
        }

        for (&id, _) in self.inner_ref().cache.iter() {
            clear_batch.push((id, None));
        }

        self.inner_ref_mut().cache.clear();
        self.inner_ref_mut().store.write_batch(clear_batch)?;
        self.inner_ref_mut().catalog.ids.clear();

        Ok(())
    }

    fn write_page<P>(&self, page: &P, id: StoreID) -> crate::Result<()>
    where
        P: Serialize,
    {
        let data = bincode::serialize(page)?;
        self.inner_ref_mut().cache.insert(id, data);

        // Periodically flush the cache when writing
        if self.inner_ref_mut().cache.len() > CACHE_SIZE {
            self.inner_ref_mut().flush_cache()?;
        }

        Ok(())
    }

    fn read_page<P>(&self, id: StoreID) -> crate::Result<Option<P>>
    where
        for<'de> P: Deserialize<'de>,
    {
        if let Some(data) = self.inner_ref().cache.get(&id) {
            return Ok(Some(bincode::deserialize(data.as_ref())?));
        }

        if let Some(data) = self.inner_ref().store.read(id)? {
            return Ok(Some(bincode::deserialize(data.as_ref())?));
        }

        Ok(None)
    }
}

trait ObjectStoreInner {
    fn inner_ref(&self) -> Ref<GlobalStoreInner>;
    fn inner_ref_mut(&self) -> RefMut<GlobalStoreInner>;
}

impl<C> ObjectStoreInner for LocalStore<C>
where
    C: Clone + Serialize,
{
    fn inner_ref(&self) -> Ref<GlobalStoreInner> {
        self.root.as_ref().borrow()
    }

    fn inner_ref_mut(&self) -> RefMut<GlobalStoreInner> {
        self.root.as_ref().borrow_mut()
    }
}

impl ObjectStoreInner for GlobalStore {
    fn inner_ref(&self) -> Ref<GlobalStoreInner> {
        self.inner.as_ref().borrow()
    }

    fn inner_ref_mut(&self) -> RefMut<GlobalStoreInner> {
        self.inner.as_ref().borrow_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_load() {
        let dir = tempfile::tempdir().unwrap();
        let _store = GlobalStore::load(&dir).unwrap();
    }

    #[test]
    fn global_reload() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();

        let id = {
            let mut store = GlobalStore::load(&dir1).unwrap();
            store.allocate_page()
        };

        {
            let mut store = GlobalStore::load(&dir1).unwrap();
            let new_id = store.allocate_page();
            assert_ne!(
                id, new_id,
                "IDAllocator did not persist allocation of a page."
            );
        }

        {
            let mut store = GlobalStore::load(&dir2).unwrap();
            let new_id = store.allocate_page();
            assert_eq!(id, new_id, "IDAllocator should be deterministic.");
        }
    }

    #[test]
    fn allocate_and_free_page() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = GlobalStore::load(dir.path()).unwrap();

        let page_id = store.allocate_page();
        assert!(page_id > 0, "Expected a valid page ID greater than zero");

        let freed = store.free_page(page_id).unwrap();
        assert!(freed, "Expected the page to be freed successfully");
    }

    #[test]
    fn write_and_read_page() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = GlobalStore::load(dir.path()).unwrap();

        let page_id = store.allocate_page();
        let test_data = "This is a test";

        // Write data to the page
        store
            .write_page(&test_data, page_id)
            .expect("Failed to write data to the page");

        // Read data back from the page
        let read_data: Option<String> = store
            .read_page(page_id)
            .expect("Failed to read data from the page");
        assert_eq!(
            read_data,
            Some(test_data.to_string()),
            "Data read from the page does not match the data written"
        );

        // Clean up
        store.free_page(page_id).unwrap();
    }

    #[test]
    fn page_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = GlobalStore::load(dir.path()).unwrap();

        // Attempt to read a page that does not exist
        let page_id = 9999; // Assume this page ID is not used
        let result: Option<Vec<u8>> = store
            .read_page(page_id)
            .expect("Failed to perform read operation");
        assert!(result.is_none(), "Expected no data for an unused page ID");
    }

    #[test]
    fn free_unallocated_page() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = GlobalStore::load(dir.path()).unwrap();

        // Attempt to free a page that was never allocated
        let page_id = 9999; // Assume this page ID is not used
        let freed = store.free_page(page_id).unwrap();
        assert!(
            !freed,
            "Expected the page to not be freed since it was never allocated"
        );
    }

    #[test]
    fn load_local_store() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = GlobalStore::load(dir.path()).unwrap();

        let _local_store: LocalStore<TestCatalog> = store.load_local_store("test").unwrap();
    }

    #[derive(Serialize, Deserialize, Default, Clone)]
    struct TestCatalog {
        entries: Vec<String>,
        id: StoreID,
    }

    #[test]
    fn local_store_multiple() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = GlobalStore::load(dir.path()).unwrap();

        for i in 0..5 {
            let mut local_store: LocalStore<TestCatalog> =
                store.load_local_store(i.to_string()).unwrap();
            local_store.catalog.id = local_store.allocate_page();
            local_store
                .write_page(&(i * i), local_store.catalog.id)
                .unwrap();
        }

        for i in 0..5 {
            let local_store: LocalStore<TestCatalog> =
                store.load_local_store(i.to_string()).unwrap();

            let value: i32 = local_store
                .read_page(local_store.catalog.id)
                .unwrap()
                .unwrap();

            assert_eq!(value, i * i, "Local store did not persist properly.");
        }
    }

    #[test]
    #[should_panic(
        expected = "Shutting down global object store, but not all local object stores have been freed!"
    )]
    fn drop_global_with_active_references() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = GlobalStore::load(dir.path()).unwrap();

        let _local_store: LocalStore<TestCatalog> = store.load_local_store("test").unwrap();

        // Should panic
        drop(store);
    }

    #[test]
    #[should_panic(expected = "Catalog `test` has already been loaded!")]
    fn no_multiple_local_stores() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = GlobalStore::load(dir.path()).unwrap();

        let _local_store_1: LocalStore<TestCatalog> = store.load_local_store("test").unwrap();
        // Should panic
        let _local_store_2: LocalStore<TestCatalog> = store.load_local_store("test").unwrap();
    }

    #[test]
    fn no_multiple_local_stores_with_drop() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = GlobalStore::load(dir.path()).unwrap();

        {
            let _local_store_1: LocalStore<TestCatalog> = store.load_local_store("test").unwrap();
        }
        {
            let _local_store_2: LocalStore<TestCatalog> = store.load_local_store("test").unwrap();
        }
    }

    #[test]
    fn catalog_update_and_retrieve() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = GlobalStore::load(dir.path()).unwrap();

        // Create and load a local store for TestCatalog
        {
            let mut local_store: LocalStore<TestCatalog> = store.load_local_store("test").unwrap();
            local_store.catalog.entries.push("Test Entry".into());
        } // LocalStore drops here, should automatically flush to disk

        // Load again and check if the updates are persistent
        {
            let local_store: LocalStore<TestCatalog> = store.load_local_store("test").unwrap();
            assert_eq!(local_store.catalog.entries.len(), 1);
            assert_eq!(local_store.catalog.entries[0], "Test Entry");
        }
    }

    #[test]
    fn corrupted_data_handling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let mut store = GlobalStore::load(path).unwrap();

        // Simulate data corruption by writing invalid data to a page
        {
            let page_id = store.allocate_page();
            let corrupted_data = b"Not a valid serialized TestCatal";
            store
                .write_page(&corrupted_data, page_id)
                .expect("Should be able to write raw bytes as corrupted data");

            // Attempt to read the corrupted data as a TestCatalog
            let read_result: crate::Result<Option<TestCatalog>> = store.read_page(page_id);
            assert!(
                read_result.is_err(),
                "Reading corrupted data should result in an error"
            );
        }
    }
}