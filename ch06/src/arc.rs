use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::atomic::{fence, AtomicUsize};

struct ArcData<T> {
    ref_count: AtomicUsize,
    data: T,
}

struct Arc<T> {
    // ヌルポインタでNoneを表現する
    ptr: NonNull<ArcData<T>>,
}

// TがSendかつSyncのときはArcもSend
unsafe impl<T: Send + Sync> Send for Arc<T> {}

// TがSendかつSyncのときはArcもSync
unsafe impl<T: Send + Sync> Sync for Arc<T> {}

// DerefMutは実装していない。Arc<T>は共有所有なので無条件に &mut T は与えられない
impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.data().data
    }
}

impl<T> Arc<T> {
    pub fn new(data: T) -> Arc<T> {
        // ArcData<T>をメモリ上に確保して参照カウントを1にする
        // メモリ領域確保のためにBoxを使う
        // Box::leakでこの領域への排他的な所有権を放棄し、ptrでの独自管理とする
        Arc {
            ptr: NonNull::from(Box::leak(Box::new(ArcData {
                ref_count: AtomicUsize::new(1),
                data,
            }))),
        }
    }

    fn data(&self) -> &ArcData<T> {
        // Arcオブジェクトが存在する限りポインタは有効だがコンパイラはそれを知らないので unsafe が必要
        // ラップして利便性を高めておく
        unsafe { self.ptr.as_ref() }
    }

    // 参照カウントが1のときのみ可変参照を渡せる
    // &mut Selfを受け取ることで同じArcを使ってたれかがTにアクセスすることがないことを保証する
    // selfではなくSelfなので Arc::get_mut(&mut a) のように呼び出す
    pub fn get_mut(arc: &mut Self) -> Option<&mut T> {
        if arc.data().ref_count.load(Relaxed) == 1 {
            fence(Acquire);
            // Arcは1つしかないので戻り値の可変参照&mut Tが存在している間は他からはデータにアクセスできない
            unsafe { Some(&mut arc.ptr.as_mut().data) }
        } else {
            None
        }
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        // 参照カウントを増やして同じポインタを使う
        // 厳密にアトミック操作の前後で行わないといけない操作はないので Relaxed でよい
        // abort()の実行は即時ではないので usize::MAX - 1 だと不十分
        if self.data().ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            std::process::abort();
        }
        Arc { ptr: self.ptr }
    }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        // ドロップ処理の全てが終わっていないといけないので Relaxed では不十分
        // 前の fetch_sub() との先行発生関係が必要
        // Acquireは 1 → 0 のときのみでよい。そのため AcqRel ではなく Release + fence(Acquire) でよい
        if self.data().ref_count.fetch_sub(1, Release) == 1 {
            // fetch_sub()の戻り値は元の値なので0になったとき
            fence(Acquire);
            unsafe {
                drop(Box::from_raw(self.ptr.as_ptr()));
            }
        }
    }
}

#[test]
fn test() {
    // ドロップされたことがわかるオブジェクトを用意する
    static NUM_DROPS: AtomicUsize = AtomicUsize::new(0);
    struct DetectDrop;
    impl Drop for DetectDrop {
        fn drop(&mut self) {
            NUM_DROPS.fetch_add(1, Relaxed);
        }
    }

    let x = Arc::new(("hello", DetectDrop));
    let y = x.clone();

    let t = std::thread::spawn(move || {
        assert_eq!(x.0, "hello");
    });

    assert_eq!(y.0, "hello");

    t.join().unwrap();

    assert_eq!(NUM_DROPS.load(Relaxed), 0);

    drop(y);

    assert_eq!(NUM_DROPS.load(Relaxed), 1);
}
