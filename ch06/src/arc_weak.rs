use std::cell::UnsafeCell;
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::atomic::{fence, AtomicUsize};

struct ArcData<T> {
    // Arcの参照カウント
    data_ref_count: AtomicUsize,
    // Weak + Arcの参照カウント
    alloc_ref_count: AtomicUsize,
    // Arcが0になったらTを解放できるようにUnsafeCellでラップ
    // TがなくてもWeakが存在できるようにOption<T>にする
    data: UnsafeCell<Option<T>>,
}

// WeakがArcDataを生存させる役割を担う
pub struct Weak<T> {
    ptr: NonNull<ArcData<T>>,
}

// ArcはWeakの役割以上のことを担う
pub struct Arc<T> {
    weak: Weak<T>,
}

unsafe impl<T: Sync + Send> Send for Weak<T> {}

unsafe impl<T: Sync + Send> Sync for Weak<T> {}

impl<T> Arc<T> {
    pub fn new(data: T) -> Arc<T> {
        Arc {
            weak: Weak {
                ptr: NonNull::from(Box::leak(Box::new(ArcData {
                    data_ref_count: AtomicUsize::new(1),
                    alloc_ref_count: AtomicUsize::new(1),
                    data: UnsafeCell::new(Some(data)),
                }))),
            },
        }
    }

    pub fn get_mut(arc: &mut Self) -> Option<&mut T> {
        // Weak<T>はupgrade()でArc<T>にアップグレードできる
        // そのため、&mut Tを返す前にArc<T>やWeak<T>がないことを確認する必要がある
        if arc.weak.data().alloc_ref_count.load(Relaxed) == 1 {
            fence(Acquire);
            // 引数arcの参照カウントは1つしかなく、それがArcであることも保証されている
            let arcdata = unsafe { arc.weak.ptr.as_mut() };
            let option = arcdata.data.get_mut();
            // Arcが存在している時点でOption<T>がNoneになることはない
            let data = option.as_mut().unwrap();
            Some(data)
        } else {
            None
        }
    }

    pub fn downgrade(arc: &Self) -> Weak<T> {
        arc.weak.clone()
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let ptr = self.weak.data().data.get();
        // Arcが存在している時点でOption<T>がNoneになることはない
        unsafe { (*ptr).as_ref().unwrap() }
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        // alloc_ref_countはこの中でインクリメントされる
        let weak = self.weak.clone();

        if weak.data().data_ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            std::process::abort();
        }
        Arc { weak }
    }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        // alloc_ref_countのデクリメントはWeakのdropで行われる
        if self.weak.data().data_ref_count.fetch_sub(1, Release) == 1 {
            fence(Acquire);
            let ptr = self.weak.data().data.get();
            // データへの参照カウントはゼロなので他の場所からアクセスすることはない
            unsafe {
                (*ptr) = None;
            }
        }
    }
}

impl<T> Weak<T> {
    fn data(&self) -> &ArcData<T> {
        // ptrが指すArcData<T>は常に存在する
        unsafe { self.ptr.as_ref() }
    }

    pub fn upgrade(&self) -> Option<Arc<T>> {
        // Arcが存在する場合のみArcにアップグレードできる
        // Weakしか存在しない場合Tはすでにドロップ済み
        let mut n = self.data().data_ref_count.load(Relaxed);
        loop {
            if n == 0 {
                return None;
            }
            assert!(n < usize::MAX);
            if let Err(e) =
                self.data()
                    .data_ref_count
                    .compare_exchange_weak(n, n + 1, Relaxed, Relaxed)
            {
                n = e;
                continue;
            }
            // インクリメントできたらArcを返す
            return Some(Arc { weak: self.clone() });
        }
    }
}

impl<T> Clone for Weak<T> {
    fn clone(&self) -> Self {
        if self.data().alloc_ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            std::process::abort();
        }
        Weak { ptr: self.ptr }
    }
}

impl<T> Drop for Weak<T> {
    fn drop(&mut self) {
        if self.data().alloc_ref_count.fetch_sub(1, Release) == 1 {
            fence(Acquire);
            unsafe {
                drop(Box::from_raw(self.ptr.as_ptr()));
            }
        }
    }
}

#[test]
fn test() {
    static NUM_DROPS: AtomicUsize = AtomicUsize::new(0);
    struct DetectDrop;
    impl Drop for DetectDrop {
        fn drop(&mut self) {
            NUM_DROPS.fetch_add(1, Relaxed);
        }
    }

    let x = Arc::new(("hello", DetectDrop));
    let y = Arc::downgrade(&x);
    let z = Arc::downgrade(&x);

    let t = std::thread::spawn(move || {
        let y = y.upgrade().unwrap();
        assert_eq!(y.0, "hello");
    });
    assert_eq!(x.0, "hello");
    t.join().unwrap();

    // まだドロップされていない
    assert_eq!(NUM_DROPS.load(Relaxed), 0);
    assert!(z.upgrade().is_some());

    drop(x);

    // ドロップ済み
    assert_eq!(NUM_DROPS.load(Relaxed), 1);
    assert!(z.upgrade().is_none());
}
