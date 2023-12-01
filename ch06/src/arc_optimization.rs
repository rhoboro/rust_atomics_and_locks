use std::borrow::Cow::Borrowed;
use std::cell::UnsafeCell;
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::atomic::{fence, AtomicUsize};

struct ArcData<T> {
    // Arcの参照カウント
    data_ref_count: AtomicUsize,
    // Weakの参照カウント
    alloc_ref_count: AtomicUsize,
    // Noneを使う必要がない
    data: UnsafeCell<ManuallyDrop<T>>,
}

pub struct Arc<T> {
    ptr: NonNull<ArcData<T>>,
}

impl<T> Arc<T> {
    pub fn new(data: T) -> Arc<T> {
        Arc {
            ptr: NonNull::from(Box::leak(Box::new(ArcData {
                data_ref_count: AtomicUsize::new(1),
                alloc_ref_count: AtomicUsize::new(1),
                data: UnsafeCell::new(ManuallyDrop::new(data)),
            }))),
        }
    }

    fn data(&self) -> &ArcData<T> {
        unsafe { self.ptr.as_ref() }
    }

    pub fn get_mut(arc: &mut Self) -> Option<&mut T> {
        // usize::MAXはロックの代わりに使う
        // AcquireはWeak::dropのReleaseデクリメントと対応する
        // アップグレードされた weak があれば次のdata_ref_count.loadで観測できる
        if arc
            .data()
            .alloc_ref_count
            .compare_exchange(1, usize::MAX, Acquire, Relaxed)
            .is_err()
        {
            return None;
        }

        // ReleaseはdowngradeのAcquireインクリメントに対応する
        // downgrade以降のdata_ref_countへの何らかの変更が上のis_uniqueの結果に影響しないようにする
        let is_unique = arc.data().data_ref_count.load(Relaxed) == 1;
        arc.data().alloc_ref_count.store(1, Release);
        if !is_unique {
            return None;
        }

        // AcquireはArc::dropのReleaseデクリメントに対応
        fence(Acquire);
        unsafe { Some(&mut *arc.data().data.get()) }
    }

    pub fn downgrade(arc: &Self) -> Weak<T> {
        let mut n = arc.data().alloc_ref_count.load(Relaxed);
        loop {
            // usize::MAXはロックの代わりに使う
            if n == usize::MAX {
                std::hint::spin_loop();
                n = arc.data().alloc_ref_count.load(Relaxed);
                continue;
            }
            assert!(n < usize::MAX - 1);
            if let Err(e) =
                arc.data()
                    .alloc_ref_count
                    .compare_exchange_weak(n, n + 1, Acquire, Relaxed)
            {
                n = e;
                continue;
            }
            return Weak { ptr: arc.ptr };
        }
    }
}

unsafe impl<T: Sync + Send> Send for Arc<T> {}

unsafe impl<T: Sync + Send> Sync for Arc<T> {}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // Arcが存在するのでデータはソン座推し、共有されていう可能性もある
        unsafe { &*self.data().data.get() }
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        // インクリメントは data_ref_count だけでよい
        if self.data().data_ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            std::process::abort()
        }
        Arc { ptr: self.ptr }
    }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        if self.data().data_ref_count.fetch_sub(1, Release) == 1 {
            fence(Acquire);
            unsafe {
                // 参照カウントは 0 なので誰もデータにアクセスしていない
                ManuallyDrop::drop(&mut *self.data().data.get());
            }
            // Arc<T>が残っていないのでWeakの参照カウントをデクリメント
            // Weak<T>を作って即座にドロップすることでデクリメントを実現
            // Weak<T>はTがなくても作れる
            drop(Weak { ptr: self.ptr })
        }
    }
}

pub struct Weak<T> {
    ptr: NonNull<ArcData<T>>,
}

impl<T> Weak<T> {
    fn data(&self) -> &ArcData<T> {
        unsafe { self.ptr.as_ref() }
    }

    pub fn upgrade(&self) -> Option<Arc<T>> {
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
            return Some(Arc { ptr: self.ptr });
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

unsafe impl<T: Sync + Send> Send for Weak<T> {}

unsafe impl<T: Sync + Send> Sync for Weak<T> {}

#[test]
fn test() {
    static NUM_DROPS: AtomicUsize = AtomicUsize::new(0);
    struct DetectDrop;
    impl Drop for DetectDrop {
        fn drop(&mut self) {
            NUM_DROPS.fetch_add(1, Relaxed);
        }
    }

    let x = crate::arc_weak::Arc::new(("hello", DetectDrop));
    let y = crate::arc_weak::Arc::downgrade(&x);
    let z = crate::arc_weak::Arc::downgrade(&x);

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
