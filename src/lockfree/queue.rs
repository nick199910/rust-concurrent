//! Michael-Scott lock-free queue.
//!
//! Usable with any number of producers and consumers.
//!
//! Michael and Scott.  Simple, Fast, and Practical Non-Blocking and Blocking Concurrent Queue
//! Algorithms.  PODC 1996.  <http://dl.acm.org/citation.cfm?id=248106>

use core::mem::{self, MaybeUninit};
use core::sync::atomic::Ordering;

use crossbeam_epoch::{unprotected, Atomic, Guard, Owned, Shared};
use crossbeam_utils::CachePadded;

/// Michael-Scott queue.
// The representation here is a singly-linked list, with a sentinel node at the front. In general
// the `tail` pointer may lag behind the actual tail. Non-sentinel nodes are either all `Data` or
// all `Blocked` (requests for data from blocked threads).
#[derive(Debug)]
pub struct Queue<T> {
    // 为了让队列的命中率更高，加了cache的行缓冲
    head: CachePadded<Atomic<Node<T>>>,
    tail: CachePadded<Atomic<Node<T>>>,
}
// 这里涉及的队列的哨兵节点没有包含任何值
#[derive(Debug)]
struct Node<T> {
    /// The slot in which a value of type `T` can be stored.
    ///
    /// The type of `data` is `MaybeUninit<T>` because a `Node<T>` doesn't always contain a `T`.
    /// For example, the sentinel node in a queue never contains a value: its slot is always empty.
    /// Other nodes start their life with a push operation and contain a value until it gets popped
    /// out. After that such empty nodes get added to the collector for destruction.
    data: MaybeUninit<T>,

    next: Atomic<Node<T>>,
}

// Any particular `T` should never be accessed concurrently, so no need for `Sync`.
// 实现Send的类型可以在线程间安全的传递其所有权
// 实现Sync的类型可以在线程间安全的共享
unsafe impl<T: Send> Sync for Queue<T> {}
unsafe impl<T: Send> Send for Queue<T> {}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        let q = Self {
            head: CachePadded::new(Atomic::null()),
            tail: CachePadded::new(Atomic::null()),
        };

        // SAFETY: We are creating a new queue, hence have sole ownership of it.
        // 创建了一个可以供所有线程共享的节点
        let sentinel = Owned::new(Node {
            data: MaybeUninit::uninit(),
            next: Atomic::null(),
        })
        .into_shared(unsafe { unprotected() });

        q.head.store(sentinel, Ordering::Relaxed);
        q.tail.store(sentinel, Ordering::Relaxed);
        q
    }
}

impl<T> Queue<T> {
    /// Create a new, empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `t` to the back of the queue, possibly waking up threads blocked on `pop()`.
    pub fn push(&self, t: T, guard: &Guard) {
        let new = Owned::new(Node {
            data: MaybeUninit::new(t),
            next: Atomic::null(),
        })
        .into_shared(guard);
        // 无锁编程这里的每一步都得考虑是否被其他的线程中断
        // 从记录的tail一直向后更新，因为要考虑到
        loop {
            // We push onto the tail, so we'll start optimistically by looking there first.
            let tail = self.tail.load(Ordering::Acquire, guard);


            // Attempt to push onto the `tail` snapshot; fails if `tail.next` has changed.
            // 把share这个指针干掉了，拿到了内部数据结构的引用
            let tail_ref = unsafe { tail.deref() };

            let next = tail_ref.next.load(Ordering::Acquire, guard);

            // If `tail` is not the actual tail, try to "help" by moving the tail pointer forward.
            // 在push之前一直尝试更新到真实的tail， 在插入之前要保证其next节点为空
            if !next.is_null() {
                let _ = self.tail.compare_exchange(
                    tail,
                    next,
                    Ordering::Release,
                    Ordering::Relaxed,
                    guard,
                );
                continue;
            }

            // looks like the actual tail; attempt to link at `tail.next`.
            // 然后将要插入的点push进去，同时尝试更新tail，这里是否更新成功都无所谓
            // tail -> new
            // 尝试更新tail的下一个点，但是
            if tail_ref
                .next
                .compare_exchange(
                    Shared::null(),
                    new,
                    Ordering::Release,
                    Ordering::Relaxed,
                    guard,
                )
                .is_ok()
            {
                // try to move the tail pointer forward.
                // 这里是尝试move 所以成功和失败其实是无所谓的
                let _ = self.tail.compare_exchange(
                    tail,
                    new,
                    Ordering::Release,
                    Ordering::Relaxed,
                    guard,
                );
                break;
            }
        }
    }

    /// Attempts to dequeue from the front.
    ///
    /// Returns `None` if the queue is observed to be empty.
    pub fn try_pop(&self, guard: &Guard) -> Option<T> {
        loop {
            // 获取当前头部节点
            let head = self.head.load(Ordering::Acquire, guard);
            // 获取头部节点的下一个节点, 这里可能是为空的
            let next = unsafe { head.deref() }.next.load(Ordering::Acquire, guard);
            // 使用`as_ref()`将`next`转换为`Option<&Node<T>>`，并将其绑定到`next_ref`
            let next_ref = unsafe { next.as_ref() }?;

            // Moves `tail` if it's stale. Relaxed load is enough because if tail == head, then the
            // messages for that node are already acquired.
            // 如果队列为空，尝试移动尾部指针到下一个节点，使用`compare_exchange`原子操作
            // 头节点等于尾节点说明队列为空
            let tail = self.tail.load(Ordering::Relaxed, guard);
            if tail == head {
                let _ = self.tail.compare_exchange(
                    tail,
                    next,
                    Ordering::Release,
                    Ordering::Relaxed,
                    guard,
                );
            }

            // 尝试更新头部指针，使用compare_exchange 原子操作
            if self
                .head
                .compare_exchange(head, next, Ordering::Release, Ordering::Relaxed, guard)
                .is_ok()
            {
                // Since the above `compare_exchange()` succeeded, `head` is detached from `self` so
                // is unreachable from  other threads.

                // SAFETY: `next` will never be the sentinel node, since it is the node after
                // `head`. Hence, it must have been a node made in `push()`, which is initialized.
                //
                // Also, we are returning ownership of `data` in `next` by making a copy of it via
                // `assume_init_read()`. This is safe as no other thread has access to `data` after
                // `head` is unreachable, so the ownership of `data` in `next` will never be used
                // again as it is now a sentinel node.
                let result = unsafe { next_ref.data.assume_init_read() };

                // SAFETY: `head` is unreachable, and we no longer access `head`. We destroy `head`
                // after the final access to `next` above to ensure that `next` is also destroyed
                // after.
                unsafe { guard.defer_destroy(head) };

                return Some(result);
            }
        }
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        // Destroy the sentinel node.

        let sentinel = mem::take(&mut *self.head);

        // Destroy and deallocate `data` for the rest of the nodes.

        // SAFETY: `pop()` never dropped the sentinel node so it is still valid.
        let mut o_curr = unsafe { sentinel.into_owned() }.into_box().next;
        // SAFETY: All non-null nodes made were valid, and we have unique ownership via `&mut self`.
        while let Some(curr) = unsafe { o_curr.try_into_owned() }.map(Owned::into_box) {
            // SAFETY: Not sentinel node, so `data` is valid.
            drop(unsafe { curr.data.assume_init() });
            o_curr = curr.next;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crossbeam_epoch::pin;
    use std::thread::scope;

    struct Queue<T> {
        queue: super::Queue<T>,
    }

    impl<T> Queue<T> {
        pub fn new() -> Queue<T> {
            Queue {
                queue: super::Queue::new(),
            }
        }

        pub fn push(&self, t: T) {
            let guard = &pin();
            self.queue.push(t, guard);
        }

        pub fn is_empty(&self) -> bool {
            let guard = &pin();
            let head = self.queue.head.load(Ordering::Acquire, guard);
            let next = unsafe { head.deref() }.next.load(Ordering::Acquire, guard);
            next.is_null()
        }

        pub fn try_pop(&self) -> Option<T> {
            let guard = &pin();
            self.queue.try_pop(guard)
        }

        pub fn pop(&self) -> T {
            loop {
                if let Some(t) = self.try_pop() {
                    return t;
                }
            }
        }
    }

    const CONC_COUNT: i64 = 1000000;

    #[test]
    fn push_try_pop_1() {
        let q: Queue<i64> = Queue::new();
        assert!(q.is_empty());
        q.push(37);
        assert!(!q.is_empty());
        assert_eq!(q.try_pop(), Some(37));
        assert!(q.is_empty());
    }

    #[test]
    fn push_try_pop_2() {
        let q: Queue<i64> = Queue::new();
        assert!(q.is_empty());
        q.push(37);
        q.push(48);
        assert_eq!(q.try_pop(), Some(37));
        assert!(!q.is_empty());
        assert_eq!(q.try_pop(), Some(48));
        assert!(q.is_empty());
    }

    #[test]
    fn push_try_pop_many_seq() {
        let q: Queue<i64> = Queue::new();
        assert!(q.is_empty());
        for i in 0..200 {
            q.push(i)
        }
        assert!(!q.is_empty());
        for i in 0..200 {
            assert_eq!(q.try_pop(), Some(i));
        }
        assert!(q.is_empty());
    }

    #[test]
    fn push_pop_1() {
        let q: Queue<i64> = Queue::new();
        assert!(q.is_empty());
        q.push(37);
        assert!(!q.is_empty());
        assert_eq!(q.pop(), 37);
        assert!(q.is_empty());
    }

    #[test]
    fn push_pop_2() {
        let q: Queue<i64> = Queue::new();
        q.push(37);
        q.push(48);
        assert_eq!(q.pop(), 37);
        assert_eq!(q.pop(), 48);
    }

    #[test]
    fn push_pop_many_seq() {
        let q: Queue<i64> = Queue::new();
        assert!(q.is_empty());
        for i in 0..200 {
            q.push(i)
        }
        assert!(!q.is_empty());
        for i in 0..200 {
            assert_eq!(q.pop(), i);
        }
        assert!(q.is_empty());
    }

    #[test]
    fn push_try_pop_many_spsc() {
        let q: Queue<i64> = Queue::new();
        assert!(q.is_empty());

        scope(|scope| {
            scope.spawn(|| {
                let mut next = 0;

                while next < CONC_COUNT {
                    if let Some(elem) = q.try_pop() {
                        assert_eq!(elem, next);
                        next += 1;
                    }
                }
            });

            for i in 0..CONC_COUNT {
                q.push(i)
            }
        });
    }

    #[test]
    fn push_try_pop_many_spmc() {
        fn recv(q: &Queue<i64>) {
            let mut cur = -1;
            for _ in 0..CONC_COUNT {
                if let Some(elem) = q.try_pop() {
                    assert!(elem > cur);
                    cur = elem;

                    if cur == CONC_COUNT - 1 {
                        break;
                    }
                }
            }
        }

        let q: Queue<i64> = Queue::new();
        assert!(q.is_empty());
        scope(|scope| {
            for _ in 0..3 {
                scope.spawn(|| recv(&q));
            }

            scope.spawn(|| {
                for i in 0..CONC_COUNT {
                    q.push(i);
                }
            });
        });
    }

    #[test]
    fn push_try_pop_many_mpmc() {
        enum LR {
            Left(i64),
            Right(i64),
        }

        let q: Queue<LR> = Queue::new();
        assert!(q.is_empty());

        scope(|scope| {
            scope.spawn(|| {
                for i in 0..CONC_COUNT {
                    q.push(LR::Left(i))
                }
            });
            scope.spawn(|| {
                for i in 0..CONC_COUNT {
                    q.push(LR::Right(i))
                }
            });
            for _ in 0..2 {
                scope.spawn(|| {
                    let mut vl = vec![];
                    let mut vr = vec![];
                    for _ in 0..CONC_COUNT {
                        match q.try_pop() {
                            Some(LR::Left(x)) => vl.push(x),
                            Some(LR::Right(x)) => vr.push(x),
                            _ => {}
                        }
                    }

                    let mut vl2 = vl.clone();
                    let mut vr2 = vr.clone();
                    vl2.sort();
                    vr2.sort();

                    assert_eq!(vl, vl2);
                    assert_eq!(vr, vr2);
                });
            }
        });
    }

    #[test]
    fn push_pop_many_spsc() {
        let q: Queue<i64> = Queue::new();

        scope(|scope| {
            scope.spawn(|| {
                let mut next = 0;
                while next < CONC_COUNT {
                    assert_eq!(q.pop(), next);
                    next += 1;
                }
            });

            for i in 0..CONC_COUNT {
                q.push(i)
            }
        });
        assert!(q.is_empty());
    }

    #[test]
    fn is_empty_dont_pop() {
        let q: Queue<i64> = Queue::new();
        q.push(20);
        q.push(20);
        assert!(!q.is_empty());
        assert!(!q.is_empty());
        assert!(q.try_pop().is_some());
    }
}
