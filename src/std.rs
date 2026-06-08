use std::{
    borrow::Borrow,
    cell::UnsafeCell,
    ptr,
    sync::{Arc, LockResult, Mutex, MutexGuard, PoisonError, TryLockError, TryLockResult},
};

/// A parent mutex that holds no data and can be used to cheaply unlock
/// its child mutexes.
#[derive(Default)]
pub struct ParentMutex(Mutex<()>);

impl ParentMutex {
    /// Create a new parent mutex.
    #[must_use]
    pub const fn new() -> Self {
        ParentMutex(Mutex::new(()))
    }

    /// Lock the parent mutex and get a key that can be used to unlock
    /// child mutexes. Blocks the current thread until the key becomes
    /// available.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent mutex is poisoned. This happens
    /// if a thread panics while holding the key, potentially violating
    /// mutex invariants. The error still contains a [`MutexKey`], which
    /// you can use to fix those invariants and clear the poison if you
    /// want. See [`Mutex`] for more details.
    pub fn key(&self) -> LockResult<MutexKey<'_>> {
        let address = ptr::from_ref(self);
        match self.0.lock() {
            Ok(guard) => Ok(MutexKey { guard, address }),
            Err(err) => Err(PoisonError::new(MutexKey {
                guard: err.into_inner(),
                address,
            })),
        }
    }

    /// Attempts to lock the parent mutex and get the key without blocking.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent mutex is poisoned (see [key][ParentMutex::key]), or if
    /// the key is already held by another thread.
    pub fn try_key(&self) -> TryLockResult<MutexKey<'_>> {
        let address = ptr::from_ref(self);
        match self.0.try_lock() {
            Ok(guard) => Ok(MutexKey { guard, address }),
            Err(TryLockError::Poisoned(err)) => {
                Err(TryLockError::Poisoned(PoisonError::new(MutexKey {
                    guard: err.into_inner(),
                    address,
                })))
            }
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
        }
    }

    /// Checks if the parent mutex is poisoned.
    ///
    /// See [`Mutex`] for details about poisoning.
    pub fn is_poisoned(&self) -> bool {
        self.0.is_poisoned()
    }

    /// Clears the poison flag on the parent mutex.
    ///
    /// See [`Mutex`] for details about poisoning.
    pub fn clear_poison(&self) {
        self.0.clear_poison();
    }
}

/// A child mutex that can be cheaply unlocked with a key from the parent.
///
/// The child mutexes refer to the parent with a generic parameter
/// `M: Borrow<ParentMutex>`, to accomodate both references and [`Arcs`][Arc].
/// This can make delaring child mutex types annoying; use [`RefChildMutex`]
/// and [`ArcChildMutex`] for type declarations.
///
/// # Example
///
/// ```
/// # use child_lock::std::*;
/// let parent = ParentMutex::new();
///
/// let child_a = ChildMutex::new(5, &parent);
/// let child_b = ChildMutex::new("foo", &parent);
///
/// // Interchangeable keys can be gotten from the parent
/// // or any of its children.
/// let key = parent.key().unwrap();
///
/// // Unlocking the child locks is essentially free;
/// // they don't have real mutexes inside them.
/// assert_eq!(*child_a.read(&key), 5);
/// assert_eq!(*child_b.read(&key), "foo");
/// ```
pub struct ChildMutex<T: ?Sized, M: Borrow<ParentMutex>> {
    parent: M,
    data: UnsafeCell<T>,
}

/// A child mutex with a plain reference to the parent.
pub type RefChildMutex<'p, T> = ChildMutex<T, &'p ParentMutex>;

/// A child mutex that references its parent through an [`Arc`],
/// with no lifetime requirements.
pub type ArcChildMutex<T> = ChildMutex<T, Arc<ParentMutex>>;

impl<T, M: Borrow<ParentMutex>> ChildMutex<T, M> {
    /// Create a new child mutex.
    pub const fn new(data: T, lock: M) -> Self {
        ChildMutex {
            parent: lock,
            data: UnsafeCell::new(data),
        }
    }

    /// Consumes `self` to produce the inner value. No key is
    /// required because you can't call this function if `self` is
    /// still shared.
    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}

impl<T: ?Sized, M: Borrow<ParentMutex>> ChildMutex<T, M> {
    /// Get a reference to the parent mutex.
    pub fn parent(&self) -> &M {
        &self.parent
    }

    /// Gets the key, blocking the current thread until it is
    /// available. Equivalent to calling [`key`][ParentMutex::key] on
    /// the parent.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent is poisoned. See [`key`][ParentMutex::key].
    pub fn key(&self) -> LockResult<MutexKey<'_>> {
        self.parent.borrow().key()
    }

    /// Tries to get the key, but returns an error without blocking if it is
    /// unavailable. Equivalent to calling [`try_key`][ParentMutex::try_key] on
    /// the parent.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent is poisoned. See [`try_key`][ParentMutex::try_key].
    pub fn try_key(&self) -> TryLockResult<MutexKey<'_>> {
        self.parent.borrow().try_key()
    }

    /// Get a shared reference to the contents of this child mutex with a key.
    ///
    /// This keeps both `self` and `key` borrowed for as long as the returned
    /// reference lives.
    ///
    /// # Panics
    ///
    /// This method will panic if the key was gotten from a parent other than
    /// this child's parent. For example:
    ///
    /// ```should_panic
    /// # use child_lock::std::{ParentMutex, ChildMutex};
    /// let parent1 = ParentMutex::new();
    /// let parent2 = ParentMutex::new();
    ///
    /// let child = ChildMutex::new(5, &parent1);
    ///
    /// // Getting key from the wrong parent!
    /// let key = parent2.key().unwrap();
    ///
    /// child.read(&key);
    /// ```
    pub fn read<'a>(&'a self, key: &'a MutexKey<'_>) -> &'a T {
        match self.try_read(key) {
            Ok(r) => r,
            Err(MismatchingMutexKeyError {
                key_parent,
                child_parent,
            }) => {
                panic!(
                    "Attempting to read a child mutex with parent at address {child_parent:?}, but with a key at address {key_parent:?}"
                );
            }
        }
    }

    /// Get a shared reference to the contents of this child mutex with a key.
    ///
    /// This keeps both `self` and `key` borrowed for as long as the returned
    /// reference lives.
    ///
    /// # Errors
    ///
    /// This method will return an error if the key was gotten from a parent other than
    /// this child's parent. For example:
    ///
    /// ```
    /// # use child_lock::std::{ParentMutex, ChildMutex};
    /// let parent1 = ParentMutex::new();
    /// let parent2 = ParentMutex::new();
    ///
    /// let child = ChildMutex::new(5, &parent1);
    ///
    /// // Getting key from the wrong parent!
    /// let key = parent2.key().unwrap();
    ///
    /// assert!(child.try_read(&key).is_err());
    /// ```
    pub fn try_read<'a>(
        &'a self,
        key: &'a MutexKey<'_>,
    ) -> Result<&'a T, MismatchingMutexKeyError> {
        let lock = self.parent.borrow();
        if key.address != ptr::from_ref(lock) {
            return Err(MismatchingMutexKeyError {
                key_parent: key.address,
                child_parent: lock,
            });
        }
        Ok(unsafe { &*self.data.get() })
    }

    /// Get an exclusive reference to the contents of this child mutex with
    /// an exclusive reference to a key.
    ///
    /// This keeps both `self` and `key` borrowed for as long as the returned
    /// reference lives.
    ///
    /// # Panics
    ///
    /// This method will panic if the key was gotten from a parent other than
    /// this child's parent. See [`read`][ChildMutex::read].
    pub fn write<'a>(&'a self, key: &'a mut MutexKey<'_>) -> &'a mut T {
        match self.try_write(key) {
            Ok(r) => r,
            Err(MismatchingMutexKeyError {
                key_parent,
                child_parent,
            }) => {
                panic!(
                    "Attempting to write a child mutex with parent at address {child_parent:?}, but with a key at address {key_parent:?}"
                );
            }
        }
    }

    /// Get an exclusive reference to the contents of this child mutex with
    /// an exclusive reference to a key.
    ///
    /// This keeps both `self` and `key` borrowed for as long as the returned
    /// reference lives.
    ///
    /// # Errors
    ///
    /// This method will return an error if the key was gotten from a parent other than
    /// this child's parent. See [`try_read`][ChildMutex::try_read].
    pub fn try_write<'a>(
        &'a self,
        key: &'a mut MutexKey<'_>,
    ) -> Result<&'a mut T, MismatchingMutexKeyError> {
        let lock = self.parent.borrow();
        if key.address != ptr::from_ref(lock) {
            return Err(MismatchingMutexKeyError {
                key_parent: key.address,
                child_parent: lock,
            });
        }
        Ok(unsafe { &mut *self.data.get() })
    }

    /// Gets a reference to the contents without a key. This is safe because
    /// it requires an exclusive reference to `self`.
    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut *self.data.get() }
    }
}

impl<T, M: Clone + Borrow<ParentMutex>> ChildMutex<T, M> {
    /// Create a new child mutex from the same parent as this child.
    ///
    /// This is especially useful with [`ArcChildMutex`], where the original
    /// parent handle might have been dropped.
    ///
    /// ```
    /// # use std::sync::Arc;
    /// # use child_lock::std::*;
    /// let child1 = ChildMutex::new(5, Arc::new(ParentMutex::new()));
    /// let child2 = child1.new_sibling("foo");
    ///
    /// let key = child1.key().unwrap();
    /// assert_eq!(*child2.read(&key), "foo");
    /// ```
    pub fn new_sibling<U>(&self, data: U) -> ChildMutex<U, M> {
        ChildMutex::new(data, self.parent.clone())
    }
}

/// A key that can unlock all the [`ChildMutexes`][ChildMutex] in a family.
///
/// Can be obtained from either the parent of any of the children.
#[expect(
    dead_code,
    reason = "The read guard is never used but it must be kept alive"
)]
pub struct MutexKey<'a> {
    guard: MutexGuard<'a, ()>,
    address: *const ParentMutex,
}

/// An error indicating that a child was unlocked with a key that came from the
/// wrong parent.
#[derive(Debug)]
pub struct MismatchingMutexKeyError {
    key_parent: *const ParentMutex,
    child_parent: *const ParentMutex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_child_mutex() {
        let mutex = ChildMutex::new(5, ParentMutex::new());

        let mut key = mutex.key().unwrap();
        assert_eq!(*mutex.write(&mut key), 5)
    }

    #[test]
    fn read_ref_child_mutex() {
        let parent = ParentMutex::new();
        let mutex = ChildMutex::new(5, &parent);

        let key = parent.key().unwrap();
        assert_eq!(*mutex.read(&key), 5);
    }

    #[test]
    fn read_arc_child_mutex() {
        let parent = Arc::new(ParentMutex::new());
        let mutex = ChildMutex::new(5, parent.clone());

        drop(parent);

        let key = mutex.key().unwrap();
        assert_eq!(*mutex.read(&key), 5);
    }

    #[test]
    fn new_sibling_ref() {
        let parent = ParentMutex::new();
        let mutex = ChildMutex::new(5, &parent);
        let sibling = mutex.new_sibling(10);

        let key = parent.key().unwrap();

        assert_eq!(*mutex.read(&key), 5);
        assert_eq!(*sibling.read(&key), 10);
    }

    #[test]
    fn new_sibling_arc() {
        let parent = Arc::new(ParentMutex::new());
        let mutex = ChildMutex::new(5, parent.clone());
        drop(parent);

        let sibling = mutex.new_sibling(10);

        let key = mutex.key().unwrap();

        assert_eq!(*mutex.read(&key), 5);
        assert_eq!(*sibling.read(&key), 10);
    }

    #[test]
    fn write() {
        let parent = ParentMutex::new();
        let mutex = ChildMutex::new(5, &parent);
        let mut key = parent.key().unwrap();
        *mutex.write(&mut key) = 10;
        assert_eq!(*mutex.read(&key), 10);
    }

    #[test]
    #[should_panic]
    fn wrong_key_panic() {
        let parent = ParentMutex::new();
        let impostor = ParentMutex::new();
        let mutex = ChildMutex::new(5, &parent);
        let key = impostor.key().unwrap();
        assert_eq!(*mutex.read(&key), 5);
    }

    #[test]
    fn try_wrong_key_err() {
        let parent = ParentMutex::new();
        let impostor = ParentMutex::new();
        let mutex = ChildMutex::new(5, &parent);
        let key = impostor.key().unwrap();
        assert!(mutex.try_read(&key).is_err());
    }
}
