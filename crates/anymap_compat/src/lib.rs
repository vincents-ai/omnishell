//! Drop-in replacement for the `anymap` crate compatible with Rust 1.88+
//!
//! Uses `HashMap<TypeId, Box<dyn Any + Send + Sync>>` with a passthrough hasher
//! (TypeId is already hashed by the compiler, no need to re-hash).

use std::any::{Any, TypeId};
use std::collections::{hash_map, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::marker::PhantomData;

/// A no-op hasher for `TypeId` since it is already heavily hashed by the compiler.
#[derive(Default)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    fn write(&mut self, _bytes: &[u8]) {
        unreachable!("TypeId calls write_u64 directly");
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = i;
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

type TypeMap = HashMap<TypeId, Box<dyn Any>, BuildHasherDefault<IdHasher>>;

/// A map keyed by type.
#[derive(Default)]
pub struct Map {
    inner: TypeMap,
}

impl Map {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T: Any>(&mut self, val: T) -> Option<T> {
        let id = TypeId::of::<T>();
        self.inner
            .insert(id, Box::new(val))
            .and_then(|v| v.downcast::<T>().ok())
            .map(|b| *b)
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        let id = TypeId::of::<T>();
        self.inner.get(&id).and_then(|v| v.downcast_ref::<T>())
    }

    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        let id = TypeId::of::<T>();
        self.inner.get_mut(&id).and_then(|v| v.downcast_mut::<T>())
    }

    pub fn contains<T: 'static>(&self) -> bool {
        self.inner.contains_key(&TypeId::of::<T>())
    }

    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        let id = TypeId::of::<T>();
        self.inner
            .remove(&id)
            .and_then(|v| v.downcast::<T>().ok())
            .map(|b| *b)
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn entry<T: Any>(&mut self) -> Entry<'_, T> {
        let id = TypeId::of::<T>();
        match self.inner.entry(id) {
            hash_map::Entry::Occupied(e) => Entry::Occupied(OccupiedEntry {
                inner: e,
                _marker: PhantomData,
            }),
            hash_map::Entry::Vacant(e) => Entry::Vacant(VacantEntry {
                inner: e,
                _marker: PhantomData,
            }),
        }
    }
}

pub enum Entry<'a, T: 'static> {
    Occupied(OccupiedEntry<'a, T>),
    Vacant(VacantEntry<'a, T>),
}

pub struct OccupiedEntry<'a, T: 'static> {
    inner: hash_map::OccupiedEntry<'a, TypeId, Box<dyn Any>>,
    _marker: PhantomData<T>,
}

pub struct VacantEntry<'a, T: 'static> {
    inner: hash_map::VacantEntry<'a, TypeId, Box<dyn Any>>,
    _marker: PhantomData<T>,
}

impl<'a, T: Any> Entry<'a, T> {
    pub fn or_insert(self, default: T) -> &'a mut T {
        match self {
            Entry::Occupied(e) => e.inner.into_mut().downcast_mut::<T>().unwrap(),
            Entry::Vacant(e) => e
                .inner
                .insert(Box::new(default))
                .downcast_mut::<T>()
                .unwrap(),
        }
    }
}

// Compatibility shim: provide the same trait that shrs_hooks expects
// so that shrs can use the same API with CloneAny bounds
pub trait CloneAny: Any + Send + Sync {
    fn clone_box(&self) -> Box<dyn CloneAny>;
}

impl<T: Any + Send + Sync + Clone + 'static> CloneAny for T {
    fn clone_box(&self) -> Box<dyn CloneAny> {
        Box::new(self.clone())
    }
}
