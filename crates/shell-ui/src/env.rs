//! Values provided to every view, one per type.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

/// What [`Ui::provide`](crate::Ui::provide) has been given, by type.
#[derive(Default)]
pub(crate) struct Env(HashMap<TypeId, Box<dyn Any>>);

impl Env {
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.0.get(&TypeId::of::<T>()).and_then(|value| value.downcast_ref())
    }

    /// Stores `value`, replacing any other of its type; returns whether that changed
    /// anything.
    pub fn set<T: PartialEq + 'static>(&mut self, value: T) -> bool {
        if self.get::<T>() == Some(&value) {
            return false;
        }
        self.0.insert(TypeId::of::<T>(), Box::new(value));
        true
    }
}
