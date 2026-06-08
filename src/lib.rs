//! `child_lock` lets you synchronize multiple "child" [`Mutexes`][::std::sync::Mutex] or
//! [`RwLocks`][::std::sync::RwLock] to a single "parent" lock. Only the parent lock contains
//! a real [`Mutex`][::std::sync::Mutex] or [`RwLock`][::std::sync::RwLock]; the children are safe wrappers around
//! [`UnsafeCell`][::std::cell::UnsafeCell] that can be cheaply unlocked after obtaining a key from the parent.
//!
//! ```
//! # use child_lock::std::*;
//! let parent = ParentMutex::new();
//!
//! let child_a = ChildMutex::new(5, &parent);
//! let child_b = ChildMutex::new("foo", &parent);
//!
//! // Interchangeable keys can be gotten from the parent
//! // or any of its children.
//! let key = parent.key().unwrap();
//!
//! // Unlocking the child locks is essentially free;
//! // they don't have real mutexes inside them.
//! assert_eq!(*child_a.read(&key), 5);
//! assert_eq!(*child_b.read(&key), "foo");
//! ```
//!
//! For `Mutex`es, only one key can exist for a family at a time. Trying to get another
//! from a different thread will block that thread, just like a normal mutex. Using
//! that key, you can access any number of child mutexes immutably simultaneously,
//! or any one child mutex mutably. (If you could mutate multiple simultaneously,
//! there would be no compile-time check to stop you from getting two exclusive
//! references to the same mutex, violating Rust's borrowing rules.)
//!
//! For `RwLocks`, you can get many [`SharedKey`s][std::SharedKey] from different threads
//! at the same time, and use them to unlock any number of child `RwLocks` immutably.
//! Or you can get one [`ExclusiveKey`][std::ExclusiveKey] with the same semantics as a [`MutexKey`][std::MutexKey] above.
//!
//! # Why would I want this?
//!
//! You probably don't. In most cases, you can just put all of the relevant data inside
//! a single `Mutex` or `RwLock`, which makes this crate useless. Even if you can't put
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
//!    the guard instead. If that isn't practical though, you can use a child mutex:
//!    ```
//!    # use child_lock::std::{ArcChildMutex, MutexKey};
//!    struct Inner;
//!    struct Outer(ArcChildMutex<Inner>);
//!    
//!    impl Outer {
//!       fn inner<'a>(&'a self, key: &'a MutexKey) -> &'a Inner {
//!          self.0.read(key)
//!       }
//!    }
//!    ```
//!
//! # `ArcChild` and `RefChild`
//!
//! The child locks need to reference the parent lock, and this can be done with any
//! type that implements `Borrow<ParentMutex>` (or `Borrow<ParentRwLock>` for [`ChildRwLocks`][std::ChildRwLock]).
//! For example, you can use `Arc<ParentMutex>` and then forget about the parent:
//!
//! ```
//! # use child_lock::std::{ParentMutex, ChildMutex};
//! # use std::sync::Arc;
//! let parent = Arc::new(ParentMutex::new());
//! let child = ChildMutex::new(5, parent.clone());
//!
//! drop(parent);
//!
//! *child.write(&mut child.key().unwrap()) = 10;
//! ```
//!
//! To make it easier to declare child lock types, use [`RefChildMutex`][std::`RefChildMutex`], [`ArcChildMutex`][std::ArcChildMutex], and the
//! equivalent types for `RwLocks`.
//!
//! # Feature flags
//!
//! ## `std` (default)
//!
//! Implements lock families for the stdlib's [`Mutex`][::std::sync::Mutex] and [`RwLock`][::std::sync::RwLock].
//!
//! **Disabling this feature does not make this crate `no_std` compatible!** The only other way to use this
//! library is with `parking_lot`, which also requires the standard library.
//!
//! ## `parking_lot`
//!
//! Equivalent lock families are available for `parking_lot`'s [`Mutex`][::parking_lot::Mutex] and [`RwLock`][::parking_lot::RwLock], though not
//! all of the `parking_lot`'s functionality is implemented.

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "std")]
pub mod std;

#[cfg(feature = "parking_lot")]
pub mod parking_lot;
