//! `child_lock` lets you synchronize multiple "child" locks
//! to a single "parent" lock, such as a [`Mutex`][::std::sync::Mutex] or [`RwLock`][::std::sync::RwLock].
//! Only the parent lock contains
//! a real lock; [`ChildLock`] is just a safe wrapper around
//! [`UnsafeCell`] that can be cheaply unlocked after obtaining a key from the parent.
//!
//! Parent lock implementations are provided for [`Mutex`][::std::sync::Mutex] and [`RwLock`][::std::sync::RwLock],
//! as well as the [`parking_lot`][::parking_lot] equivalents if the `parking_lot` feature is enabled.
//!
//! # Example
//!
//! ```
//! # use child_lock::ChildLock;
//! # use child_lock::std::*;
//! let parent = MutexParent::new();
//!
//! let child_a = ChildLock::new(5, &parent);
//! let child_b = ChildLock::new("foo", &parent);
//!
//! // Interchangeable keys can be gotten from the parent
//! // or any of its children.
//! let key = parent.key().unwrap();
//!
//! // Unlocking the child locks is essentially free;
//! // they don't have real locks inside them.
//! assert_eq!(*child_a.read(&key), 5);
//! assert_eq!(*child_b.read(&key), "foo");
//! ```
//!
//! For [`MutexParents`][std::MutexParent], only one key can exist for a family at a time. Trying to get another
//! from a different thread will block that thread, just like a normal mutex. Using
//! that key, you can access any number of child mutexes immutably simultaneously,
//! or any one child mutex mutably. (If you could mutate multiple simultaneously,
//! there would be no compile-time check to stop you from getting two exclusive
//! references to the same mutex, violating Rust's borrowing rules.)
//!
//! For [`RwParents`][std::RwParent], you can get many [`RwLockReadKeys`][std::RwLockReadKey] from different threads
//! at the same time, and use them to unlock any number of child `RwLocks` immutably.
//! Or you can get one [`RwLockWriteKey`][std::RwLockWriteKey] with the same semantics as a [`MutexKey`][std::MutexKey] above.
//!
//! # Why would I want this?
//!
//! You probably don't. In most cases, you can just put all of the relevant data inside
//! a single [`Mutex`][::std::sync::Mutex] or [`RwLock`][::std::sync::RwLock], which
//! makes this crate useless. Even if you can't put
//! everything in a single lock, this crate might still be a bad idea; you can't write
//! to more than one child lock at a time. However, if you only need to *read* from
//! many related locks at the same time, `child_lock` might have some advantages.
//!
//! 1. **Performance:** Only the parent lock has an actual lock inside it. If the
//!    speed of unlocking many related locks is a concern, with `child_lock` you
//!    only pay the cost of a single lock. Using the key on a child lock only costs
//!    a pointer comparison to make sure you are using the correct key.
//! 2. **Easier to reference the contents of a child lock:** Say you want to want to
//!    return a reference to the contents of a mutex like this:
//!    ```compile_fail
//!    # use std::sync::Mutex;
//!    struct Inner;
//!    struct Outer(Mutex<Inner>);
//!
//!    impl Outer {
//!        fn inner(&self) -> &Inner {
//!            let guard = self.0.lock().unwrap();
//!            // Error: cannot return value referencing local variable `guard`
//!            &*guard
//!        }
//!    }
//!    ```
//!    This fails because the `&Inner` reference can't outlive the mutex guard, which
//!    is dropped at the end of `fn inner`. In most cases, the solution is to return
//!    the guard instead. If that isn't practical though, you can use a child lock:
//!    ```
//!    # use child_lock::{Key, ChildLock};
//!    # use child_lock::std::MutexParent;
//!    # use std::sync::Arc;
//!    struct Inner;
//!    struct Outer(ChildLock<Inner, Arc<MutexParent>>);
//!
//!    impl Outer {
//!       fn inner<'a>(&'a self, key: &'a impl Key) -> &'a Inner {
//!          self.0.read(key)
//!       }
//!    }
//!    ```
//!
//! # Referencing the parent
//!
//! The child locks need to reference the parent lock, and this can be done with any
//! type that implements `Borrow<dyn ParentLock>`.
//! For example, you can use `Arc<MutexParent>` as the parent and then drop the original handle.
//! Then, the only way to get a key is through one of the children.
//!
//! ```
//! # use child_lock::std::{MutexParent};
//! # use child_lock::ChildLock;
//! # use std::sync::Arc;
//! let parent = Arc::new(MutexParent::new());
//! let child = ChildLock::new(5, parent.clone());
//!
//! drop(parent);
//!
//! let mut key = child.parent.key().unwrap();
//! *child.write(&mut key) = 10;
//! ```
//!
//! # Feature flags
//!
//! ## `std` (default)
//!
//! Implements parent locks for the stdlib's [`Mutex`][::std::sync::Mutex] and [`RwLock`][::std::sync::RwLock].
//!
//! **Disabling this feature does not make this crate `no_std` compatible!** The only other way to use this
//! library is with `parking_lot`, which also requires the standard library.
//!
//! ## `parking_lot`
//!
//! Equivalent parent lock implementations are available for `parking_lot`'s [`Mutex`][::parking_lot::Mutex] and
//! [`RwLock`][::parking_lot::RwLock].

#![cfg_attr(docsrs, feature(doc_cfg))]

use ::std::{
    borrow::Borrow,
    cell::UnsafeCell,
    fmt::{self, Formatter},
    ptr,
};

#[cfg(feature = "std")]
pub mod std;

#[cfg(feature = "parking_lot")]
pub mod parking_lot;

/// A trait for parent locks that can be used with [`ChildLock`].
pub trait ParentLock {
    fn marker(&self) -> &ParentMarker;
}

#[derive(Default)]
pub struct ParentMarker;

impl fmt::Debug for ParentMarker {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "ParentMarker({self:p})")
    }
}

/// A child mutex that can be cheaply unlocked with a key from the parent.
///
/// The child mutexes refer to the parent with a generic parameter
/// `M: Borrow<dyn ParentLock>`, to accomodate multiple ways of storing the parent.
/// For a given parent lock type like [`RwParent`][std::RwParent], you can use one of
/// the following, depending on how the parent is owned:
/// - `ChildLock<T, RwParent>`
/// - `ChildLock<T, &'p RwParent>`
/// - `ChildLock<T, Arc<RwParent>>`
///
/// # Example
///
/// ```
/// # use child_lock::std::*;
/// # use child_lock::ChildLock;
/// let parent = MutexParent::new();
///
/// let child_a = ChildLock::new(5, &parent);
/// let child_b = ChildLock::new("foo", &parent);
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
pub struct ChildLock<T: ?Sized, M: Borrow<dyn ParentLock>> {
    pub parent: M,
    data: UnsafeCell<T>,
}

impl<T, M: Borrow<dyn ParentLock>> ChildLock<T, M> {
    /// Create a new child mutex.
    pub const fn new(data: T, parent: M) -> Self {
        ChildLock {
            parent,
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

impl<T: ?Sized, M: Borrow<dyn ParentLock>> ChildLock<T, M> {
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
    /// # use child_lock::ChildLock;
    /// # use child_lock::std::MutexParent;
    /// let parent1 = MutexParent::new();
    /// let parent2 = MutexParent::new();
    ///
    /// let child = ChildLock::new(5, &parent1);
    ///
    /// // Getting key from the wrong parent!
    /// let key = parent2.key().unwrap();
    ///
    /// child.read(&key);
    /// ```
    pub fn read<'a>(&'a self, key: &'a impl Key) -> &'a T {
        match self.try_read(key) {
            Ok(r) => r,
            Err(MismatchingKeyError {
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
    /// # use child_lock::ChildLock;
    /// # use child_lock::std::MutexParent;
    /// let parent1 = MutexParent::new();
    /// let parent2 = MutexParent::new();
    ///
    /// let child = ChildLock::new(5, &parent1);
    ///
    /// // Getting key from the wrong parent!
    /// let key = parent2.key().unwrap();
    ///
    /// assert!(child.try_read(&key).is_err());
    /// ```
    pub fn try_read<'a>(&'a self, key: &'a impl Key) -> Result<&'a T, MismatchingKeyError<'a, 'a>> {
        let lock = self.parent.borrow();
        if !ptr::eq(key.parent_marker(), lock.marker()) {
            return Err(MismatchingKeyError {
                key_parent: key.parent_marker(),
                child_parent: lock.marker(),
            });
        }
        // SAFETY: The key's parent marker matches the child's parent marker,
        // so we know the key came from the correct parent.
        // Beyond that, the key trait implementation is responsible for
        // guaranteeing that the key uphold's Rusts borrowing rules.
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
    /// this child's parent. See [`read`][ChildLock::read].
    pub fn write<'a>(&'a self, key: &'a mut impl ExclusiveKey) -> &'a mut T {
        match self.try_write(key) {
            Ok(r) => r,
            Err(MismatchingKeyError {
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
    /// this child's parent. See [`try_read`][ChildLock::try_read].
    pub fn try_write<'a>(
        &'a self,
        key: &'a mut impl ExclusiveKey,
    ) -> Result<&'a mut T, MismatchingKeyError<'a, 'a>> {
        let lock = self.parent.borrow();
        if !ptr::eq(key.parent_marker(), lock.marker()) {
            return Err(MismatchingKeyError {
                key_parent: key.parent_marker(),
                child_parent: lock.marker(),
            });
        }
        // SAFETY: The key's parent marker matches the child's parent marker,
        // so we know the key came from the correct parent.
        // Beyond that, the key trait implementation is responsible for
        // guaranteeing that the key uphold's Rusts borrowing rules.
        Ok(unsafe { &mut *self.data.get() })
    }

    /// Gets a reference to the contents without a key. This is safe because
    /// it requires an exclusive reference to `self`.
    pub fn get_mut(&mut self) -> &mut T {
        self.data.get_mut()
    }
}

impl<T, M: Clone + Borrow<dyn ParentLock>> ChildLock<T, M> {
    /// Create a new child mutex from the same parent as this child.
    ///
    /// ```
    /// # use std::sync::Arc;
    /// # use child_lock::std::*;
    /// # use child_lock::ChildLock;
    /// let parent = Arc::new(MutexParent::new());
    /// let child1 = ChildLock::new(5, parent.clone());
    /// let child2 = child1.new_sibling("foo");
    ///
    /// let key = parent.key().unwrap();
    /// assert_eq!(*child2.read(&key), "foo");
    /// ```
    pub fn new_sibling<U>(&self, data: U) -> ChildLock<U, M> {
        ChildLock::new(data, self.parent.clone())
    }
}

// SAFETY: Only one mutable reference to the contents can exist at a time, so
// ChildLock is sync if T is sync.
unsafe impl<T: ?Sized + Send + Sync, M: Borrow<dyn ParentLock> + Send + Sync> Sync
    for ChildLock<T, M>
{
}

/// A trait for keys that can be used to immutably unlock child locks.
///
/// # Safety
///
/// Any number of keys can coexist from the same parent, as long as they
/// do NOT implement [`ExclusiveKey`]. This is impossible to enforce with
/// the type system, so the implementor must ensure this invariant is upheld.
///
/// Failure to uphold this invariant will result in violating Rust's borrowing
/// rules for the child lock data.
pub unsafe trait Key {
    fn parent_marker(&self) -> &ParentMarker;
}

/// A trait for keys that can be used to exclusively unlock child locks.
///
/// # Safety
///
/// If an [`ExclusiveKey`] exists for a given parent, no other keys can exist
/// from that parent. The implementor must ensure this invariant is upheld at runtime.
///
/// Failure to uphold this invariant will result in violating Rust's borrowing
/// rules for the child lock data.
pub unsafe trait ExclusiveKey: Key {}

/// An error indicating that a child was unlocked with a key that came from the
/// wrong parent.
#[derive(Debug)]
pub struct MismatchingKeyError<'a, 'b> {
    key_parent: &'a ParentMarker,
    child_parent: &'b ParentMarker,
}
