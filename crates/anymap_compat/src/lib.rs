//! Drop-in replacement for the `anymap` crate compatible with Rust 1.88+
//!
//! Uses `HashMap<TypeId, Box<dyn Any>>` under the hood.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// A map keyed by type.
#[derive(Default)]
pub struct Map {
    inner: HashMap<TypeId, Box<dyn Any>>,
}

impl Map {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T: 'static>(&mut self, val: T) -> Option<T> {
        let id = TypeId::of::<T>();
        self.inner
            .insert(id, Box::new(val))
            .and_then(|v| v.downcast::<T>().ok())
            .map(|b| *b)
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        let id = TypeId::of::<T>();
        self.inner
            .get(&id)
            .and_then(|v| v.downcast_ref::<T>())
    }

    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        let id = TypeId::of::<T>();
        self.inner
            .get_mut(&id)
            .and_then(|v| v.downcast_mut::<T>())
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

    pub fn entry<T: 'static>(&mut self) -> Entry<'_, T> {
        let id = TypeId::of::<T>();
        if self.inner.contains_key(&id) {
            Entry::Occupied(OccupiedEntry {
                inner: self.inner.get_mut(&id).unwrap(),
                _marker: std::marker::PhantomData,
            })
        } else {
            Entry::Vacant(VacantEntry {
                map: &mut self.inner,
                _marker: std::marker::PhantomData,
            })
        }
    }
}

pub enum Entry<'a, T: 'static> {
    Occupied(OccupiedEntry<'a, T>),
    Vacant(VacantEntry<'a, T>),
}

pub struct OccupiedEntry<'a, T: 'static> {
    inner: &'a mut Box<dyn Any>,
    _marker: std::marker::PhantomData<T>,
}

pub struct VacantEntry<'a, T: 'static> {
    map: &'a mut HashMap<TypeId, Box<dyn Any>>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: 'static> Entry<'a, T> {
    pub fn or_insert(self, default: T) -> &'a mut T {
        match self {
            Entry::Occupied(e) => e.inner.downcast_mut::<T>().unwrap(),
            Entry::Vacant(e) => {
                let id = TypeId::of::<T>();
                e.map.insert(id, Box::new(default));
                e.map.get_mut(&id).unwrap().downcast_mut::<T>().unwrap()
            }
        }
    }
}

// Compatibility shim: provide the same trait that anymap's CloneAny provides
// so that shrs_hooks can use the same API
pub trait CloneAny: Any + Send + Sync {
    fn clone_box(&self) -> Box<dyn CloneAny>;
}

impl<T: Any + Send + Sync + Clone + 'static> CloneAny for T {
    fn clone_box(&self) -> Box<dyn CloneAny> {
        Box::new(self.clone())
    }
}

// The original anymap used `anymap::Map` with `CloneAny` in some places.
// This module re-exports the same types shrs expects.
