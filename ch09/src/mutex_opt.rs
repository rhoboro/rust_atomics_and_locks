use atomic_wait::{wait, wake_one};
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

pub struct Mutex<T> {
    /// 0: unlocked
    /// 1: locked: 他の待機スレッドなし
    /// 2: locked: 他の待機スレッドあり
    state: AtomicU32,
    value: UnsafeCell<T>,
}

unsafe impl<T> Sync for Mutex<T> where T: Send {}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            state: AtomicU32::new(0),
            value: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> MutexGuard<T> {
        // ロックされていなかったら1にする
        if self.state.compare_exchange(0, 1, Acquire, Relaxed).is_err() {
            // すでにロックされていた場合はスリープする前に2にする
            // wakeされた場合は0になっているので2に戻す
            while self.state.swap(2, Acquire) != 0 {
                wait(&self.state, 2);
            }
        }
        MutexGuard { mutex: self }
    }
}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

unsafe impl<T> Sync for MutexGuard<'_, T> where T: Sync {}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        if self.mutex.state.swap(0, Release) == 2 {
            // 2の場合のみwakeする
            // 起こされた時には 0 になっている
            wake_one(&self.mutex.state);
        }
    }
}
