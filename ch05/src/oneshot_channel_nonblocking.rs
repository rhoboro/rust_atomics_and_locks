use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Acquire, Release};
use std::thread;
use std::thread::Thread;

pub struct Channel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    ready: AtomicBool,
}

unsafe impl<T> Sync for Channel<T> where T: Send {}

impl<T> Channel<T> {
    pub const fn new() -> Self {
        Self {
            message: UnsafeCell::new(MaybeUninit::uninit()),
            ready: AtomicBool::new(false),
        }
    }

    // ChannelからSenderとReceiverを作成する
    // 排他的借用（&mut Channel)にすることで同じチャネルに対して複数のSenderとReceiverが作れないことを保証
    // SenderとReceiverがドロップされた後はもう一度split()を呼び出せる
    // ライフタイムを省略しない場合はこうなる
    // pub fn split<'a>(&'a mut self) -> (Sender<'a, T>, Receiver<'a, T>) {
    pub fn split(&mut self) -> (Sender<T>, Receiver<T>) {
        // 上書きすることで古い*selfのDropが実行される
        *self = Self::new();
        (
            Sender {
                channel: self,
                receiving_thread: thread::current(),
            },
            Receiver {
                channel: self,
                _no_send: PhantomData,
            },
        )
    }
}

impl<T> Drop for Channel<T> {
    fn drop(&mut self) {
        if *self.ready.get_mut() {
            unsafe { self.message.get_mut().assume_init_drop() }
        }
    }
}

pub struct Sender<'a, T> {
    channel: &'a Channel<T>,
    receiving_thread: Thread,
}

impl<T> Sender<'_, T> {
    // 値渡しにより1度しか呼ばれないことが保証されているのでパニックしない
    pub fn send(self, message: T) {
        unsafe { (*self.channel.message.get()).write(message) };
        self.channel.ready.store(true, Release);
        self.receiving_thread.unpark();
    }
}

pub struct Receiver<'a, T> {
    channel: &'a Channel<T>,
    _no_send: PhantomData<*const ()>,
}

impl<T> Receiver<'_, T> {
    pub fn receive(self) -> T {
        // falseに戻すことで値がないことをドロップに伝えられる
        while !self.channel.ready.swap(false, Acquire) {
            thread::park();
        }
        unsafe { (*self.channel.message.get()).assume_init_read() }
    }
}