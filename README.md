# child_lock

[Read the docs](https://docs.rs/child_lock)

`child_lock` lets you synchronize multiple "child" locks
to a single "parent" lock, such as a `Mutex` or `RwLock`.
Only the parent lock contains
a real lock; `ChildLock` is just a safe wrapper around
`UnsafeCell` that can be cheaply unlocked after obtaining a key from the parent.

Parent lock implementations are provided for the standard library's `Mutex` and `RwLock`,
as well as the [`parking_lot`](https://docs.rs/parking_lot) equivalents if the `parking_lot` feature is enabled.

## Example

```rust
let parent = MutexParent::new();

let child_a = ChildLock::new(5, &parent);
let child_b = ChildLock::new("foo", &parent);

// Interchangeable keys can be gotten from the parent
// or any of its children.
let key = parent.key().unwrap();

// Unlocking the child locks is essentially free;
// they don't have real locks inside them.
assert_eq!(*child_a.read(&key), 5);
assert_eq!(*child_b.read(&key), "foo");
```
