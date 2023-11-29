use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

pub struct Channel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    // 複数のsend()が同じセルに同時にアクセスすることを防ぐ
    in_use: AtomicBool,
    ready: AtomicBool,
}

// TがSendであればこのChannelはスレッド間で共有しても安全
unsafe impl<T> Sync for Channel<T> where T: Send {}

impl<T> Channel<T> {
    pub const fn new() -> Self {
        Self {
            message: UnsafeCell::new(MaybeUninit::uninit()),
            in_use: AtomicBool::new(false),
            ready: AtomicBool::new(false),
        }
    }

    pub unsafe fn send(&self, message: T) {
        // 2つい上のメッセージを送信しようとしたらパニック
        if self.in_use.swap(true, Relaxed) {
            panic!("can't send more than one message!");
        }
        unsafe { (*self.message.get()).write(message) };
        self.ready.store(true, Release);
    }

    pub fn is_ready(&self) -> bool {
        // 同期の役割はreceiveが担うようになった
        self.ready.load(Relaxed)
    }

    // unsafeでなくなった
    pub fn receive(&self) -> T {
        if !self.ready.swap(false, Acquire) {
            panic!("no message available!");
        }
        // readyフラグを確認しリセット済みなので安全
        unsafe { (*self.message.get()).assume_init_read() }
    }
}

impl<T> Drop for Channel<T> {
    fn drop(&mut self) {
        // オブジェクトがドロップされるのはそのオブジェクトを完全に所有していて
        // ほかに借用がない場合のみに限られる
        // したがってアトミック操作は不要
        if *self.ready.get_mut() {
            // messageも同じ
            unsafe { self.message.get_mut().assume_init_drop() }
        }
    }
}
