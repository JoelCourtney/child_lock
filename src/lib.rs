use std::{
    borrow::Borrow, cell::UnsafeCell, ptr, sync::{Arc, LockResult, Mutex, MutexGuard, PoisonError, TryLockError, TryLockResult}
};

#[derive(Default)]
pub struct ParentMutex(Mutex<()>);

impl ParentMutex {
    #[must_use]
    pub const fn new() -> Self {
        ParentMutex(Mutex::new(()))
    }

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
}

pub struct ChildMutex<T: ?Sized, M: Borrow<ParentMutex>> {
    lock: M,
    data: UnsafeCell<T>,
}

pub type RefChildMutex<'m, T> = ChildMutex<T, &'m ParentMutex>;
pub type ArcChildMutex<T> = ChildMutex<T, Arc<ParentMutex>>;

impl<T, M: Borrow<ParentMutex>> ChildMutex<T, M> {
    pub fn new(data: T, lock: M) -> Self {
        ChildMutex {
            lock,
            data: UnsafeCell::new(data),
        }
    }

    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}

impl<T: ?Sized, M: Borrow<ParentMutex>> ChildMutex<T, M> {
    pub fn key(&self) -> LockResult<MutexKey<'_>> {
        self.lock.borrow().key()
    }

    pub fn try_key(&self) -> TryLockResult<MutexKey<'_>> {
        self.lock.borrow().try_key()
    }

    pub fn read<'a>(&'a self, key: &'a MutexKey<'_>) -> &'a T {
        let lock = self.lock.borrow();
        assert_eq!(
            key.address, ptr::from_ref(lock),
            "Attempted to read a child mutex with a key that came from an unrelated parent mutex."
        );
        unsafe { &*self.data.get() }
    }

    pub fn write<'a>(&'a self, key: &'a mut MutexKey<'_>) -> &'a mut T {
        let lock = self.lock.borrow();
        assert_eq!(
            key.address, ptr::from_ref(lock),
            "Attempted to write to a child mutex with a key that came from an unrelated parent mutex."
        );
        unsafe { &mut *self.data.get() }
    }

    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut *self.data.get() }
    }
    
    pub fn central_mutex_mut(&mut self) -> &mut M {
        &mut self.lock
    }
}

impl<T, M: Clone + Borrow<ParentMutex>> ChildMutex<T, M> {
    pub fn new_sibling<U>(&self, data: U) -> ChildMutex<U, M> {
        ChildMutex::new(data, self.lock.clone())
    }
}

#[expect(
    dead_code,
    reason = "The read guard is never used but it must be kept alive"
)]
pub struct MutexKey<'a> {
    guard: MutexGuard<'a, ()>,
    address: *const ParentMutex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_common_mutex() {
        let mutex = ChildMutex::new(5, ParentMutex::new());
        
        let mut key = mutex.key().unwrap();
        assert_eq!(*mutex.write(&mut key), 5)
    }

    #[test]
    fn read_ref_common_mutex() {
        let central = ParentMutex::new();
        let mutex = ChildMutex::new(5, &central);

        let key = central.key().unwrap();
        assert_eq!(*mutex.read(&key), 5);
    }

    #[test]
    fn read_arc_common_mutex() {
        let central = Arc::new(ParentMutex::new());
        let mutex = ChildMutex::new(5, central.clone());

        drop(central);

        let key = mutex.key().unwrap();
        assert_eq!(*mutex.read(&key), 5);
    }

    #[test]
    fn new_sibling_ref() {
        let central = ParentMutex::new();
        let mutex = ChildMutex::new(5, &central);
        let sibling = mutex.new_sibling(10);

        let key = central.key().unwrap();

        assert_eq!(*mutex.read(&key), 5);
        assert_eq!(*sibling.read(&key), 10);
    }

    #[test]
    fn new_sibling_arc() {
        let central = Arc::new(ParentMutex::new());
        let mutex = ChildMutex::new(5, central.clone());
        drop(central);
        
        let sibling = mutex.new_sibling(10);

        let key = mutex.key().unwrap();

        assert_eq!(*mutex.read(&key), 5);
        assert_eq!(*sibling.read(&key), 10);
    }

    #[test]
    fn write() {
        let central = ParentMutex::new();
        let mutex = ChildMutex::new(5, &central);
        let mut key = central.key().unwrap();
        *mutex.write(&mut key) = 10;
        assert_eq!(*mutex.read(&key), 10);
    }
}
