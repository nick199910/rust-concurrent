use std::cmp;
use std::mem;
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::ptr;
use std::sync::atomic::Ordering;

use crate::ConcurrentSet;
use crossbeam_epoch::{Atomic, CompareExchangeError, Guard, Owned, Shared};
use crate::lock::seqlock::{ReadGuard, SeqLock, WriteGuard};

#[derive(Debug)]
struct Node<T> {
    data: T,
    next: SeqLock<Atomic<Node<T>>>,
}

/// Concurrent sorted singly linked list using fine-grained optimistic locking
#[derive(Debug)]
pub struct OptimisticFineGrainedListSet<T: std::fmt::Display> {
    head: SeqLock<Atomic<Node<T>>>,
}

unsafe impl<T: Send + std::fmt::Display> Send for OptimisticFineGrainedListSet<T> {}
unsafe impl<T: Send + std::fmt::Display> Sync for OptimisticFineGrainedListSet<T> {}

#[derive(Debug)]
struct Cursor<'g, T: std::fmt::Display> {
    // reference to the `next` field of previous node which points to the current node
    prev: ReadGuard<'g, Atomic<Node<T>>>,
    curr: Shared<'g, Node<T>>,
}

impl<T: std::fmt::Display> Node<T> {
    fn new(data: T, next: Shared<Self>) -> Owned<Self> {
        Owned::new(Self {
            data,
            next: SeqLock::new(Atomic::from(next)),
        })
    }
}

impl<'g, T: Ord + std::fmt::Display> Cursor<'g, T> {
    /// Moves the cursor to the position of key in the sorted list.
    /// Returns whether the value was found.
    fn find(&mut self, key: &T, guard: &'g Guard) -> Result<bool, ()> {
        // Finding phase
        // - cursor.curr: first unmarked node w/ key >= search key (4)
        // - cursor.prev: the ref of .next in previous unmarked node (1 -> 2)
        // 1 -> 2 -x-> 3 -x-> 4 -> 5 -> ∅  (search key: 4)
        let mut prev_next = self.curr;
        let found = loop {
            let Some(curr_node) = (unsafe { self.curr.as_ref() }) else {
                break false;
            };
            let mut next_guard = unsafe { curr_node.next.read_lock() };
            let next = next_guard.load(Ordering::Acquire, guard);

            // - finding stage is done if cursor.curr advancement stops
            // - advance cursor.curr if (.next is marked) || (cursor.curr < key)
            // - stop cursor.curr if (not marked) && (cursor.curr >= key)
            // - advance cursor.prev if not marked
            // 如果被标记了，则找的时候就没必要再比较了，直接next即可
            if next.tag() != 0 {
                // We add a 0 tag here so that `self.curr`s tag is always 0.
                self.curr = next.with_tag(0);
                next_guard.finish();
                continue;
            }

            match curr_node.data.cmp(key) {
                cmp::Ordering::Less => {
                    self.curr = next;
                    mem::swap(&mut self.prev, &mut next_guard);
                    prev_next = next;
                    next_guard.finish();
                }
                cmp::Ordering::Equal => {
                    next_guard.finish();
                    break true;
                }
                cmp::Ordering::Greater => {
                    next_guard.finish();
                    break false;
                }
            }
        };

        // If prev and curr WERE adjacent, no need to clean up
        if prev_next == self.curr {
            return Ok(found);
        }

        // cleanup marked nodes between prev and curr
        self.prev
            .compare_exchange(
                prev_next,
                self.curr,
                Ordering::Release,
                Ordering::Relaxed,
                guard,
            )
            .map_err(|_| ())?;

        // defer_destroy from cursor.prev.load() to cursor.curr (exclusive) // 这里不是太懂
        let mut node = prev_next;
        while node.with_tag(0) != self.curr {
            // SAFETY: All nodes in the unlinked chain are not null.
            let next_guard = unsafe { node.deref().next.read_lock() };
            let next = next_guard.load(Ordering::Relaxed, guard);
            // SAFETY: we unlinked the chain with above CAS.
            unsafe { guard.defer_destroy(node) };
            node = next;
            next_guard.finish();
        }

        Ok(found)
    }
}

impl<T: std::fmt::Display> OptimisticFineGrainedListSet<T> {
    /// Creates a new list.
    pub fn new() -> Self {
        Self {
            head: SeqLock::new(Atomic::null()),
        }
    }

    fn head<'g>(&'g self, guard: &'g Guard) -> Cursor<'g, T> {
        let prev = unsafe { self.head.read_lock() };
        let curr = prev.load(Ordering::SeqCst, guard);
        Cursor { prev, curr }
    }
}

impl<T: Ord + std::fmt::Display> OptimisticFineGrainedListSet<T> {
    fn find<'g>(&'g self, key: &T, guard: &'g Guard) -> Result<(bool, Cursor<'g, T>), ()> {
        loop {
            let mut cur = self.head(guard);
            if let Ok(r) = cur.find(key, guard) {
                return Ok((r, cur));
            }
        }
    }
}

impl<T: Ord + std::fmt::Debug + std::fmt::Display> ConcurrentSet<T>
for OptimisticFineGrainedListSet<T>
{
    fn contains(&self, key: &T) -> bool {
        // Pin the current thread.
        let guard = crossbeam_epoch::pin();
        let (found, cursor) = self.find(key, &guard).unwrap();
        cursor.prev.finish();
        found
    }

    // 未insert 1 list状态
    // .
    // Atomic::null
    // 首先用find找到插入点，箭头就是
    // ⬇︎ head
    // Atomic::null
    // 然后将它放到New Node的next字段
    // Node<1>.next = Atomic::null
    // 然后交换prev指针指向新的Node
    // ⬇︎ prev
    // Node<1>
    // 然后 更新cursor.curr指针值，然后返回 cursor.curr = node;
    // 然后插入2的时候，cursor指向
    //                 ⬇︎
    // Node<1>.next = Atomic::null
    // 然后插入2，然后把null放在后面
    // Node<1>.next => Node<2> => Atomic::null
    // 然后更新cursor.curr为2
    // 如此类推
    fn insert(&self, key: T) -> bool {
        let guard = crossbeam_epoch::pin();
        let mut node = Node::new(key, Shared::null());
        loop {
            let (found, mut cursor) = self.find(&node.data, &guard).unwrap();
            if found {
                cursor.prev.finish();
                return false;
            }
            node.next.write_lock().store(cursor.curr, Ordering::Relaxed);
            match cursor.prev.compare_exchange(
                cursor.curr,
                node,
                Ordering::Release,
                Ordering::Relaxed,
                &guard,
            ) {
                Ok(node) => {
                    cursor.curr = node;
                    cursor.prev.finish();
                    return true;
                }
                Err(e) => node = e.new,
            }
            cursor.prev.finish();
        }
    }

    fn remove(&self, key: &T) -> bool {
        let guard = crossbeam_epoch::pin();
        loop {
            let (found, cursor) = self.find(key, &guard).unwrap();
            if !found {
                cursor.prev.finish();
                return false;
            }

            // SAFETY: curr was found, hence cannot be null.
            let curr_node = unsafe { cursor.curr.deref() };

            // Release: to release current view of the deleting thread on this mark.
            // Acquire: to ensure that if the latter CAS succeeds, then the thread that reads `next` through `prev` will be safe.
            let next = curr_node
                .next
                .write_lock()
                .fetch_or(1, Ordering::AcqRel, &guard);
            if next.tag() == 1 {
                cursor.prev.finish();
                continue;
            }

            if cursor
                .prev
                .compare_exchange(
                    cursor.curr,
                    next,
                    Ordering::Release,
                    Ordering::Relaxed,
                    &guard,
                )
                .is_ok()
            {
                // SAFETY: we are unlinker of curr. As the lifetime of the guard extends to the return
                // value of the function, later access of curr_node is ok.
                unsafe { guard.defer_destroy(cursor.curr) };
            }
            cursor.prev.finish();
            return true;
        }
    }
}

#[derive(Debug)]
pub struct Iter<'g, T: std::fmt::Display> {
    // Can be dropped without validation, because the only way to use cursor.curr is next().
    cursor: ManuallyDrop<Cursor<'g, T>>,
    guard: &'g Guard,
}

impl<T: std::fmt::Display> OptimisticFineGrainedListSet<T> {
    /// An iterator visiting all elements. `next()` returns `Some(Err(()))` when validation fails.
    /// In that case, further invocation of `next()` returns `None`, and the user must restart the
    /// iteration.
    pub fn iter<'g>(&'g self, guard: &'g Guard) -> Iter<'_, T> {
        Iter {
            cursor: ManuallyDrop::new(self.head(guard)),
            guard,
        }
    }
}

impl<'g, T: std::fmt::Display> Iterator for Iter<'g, T> {
    type Item = Result<&'g T, ()>;

    fn next(&mut self) -> Option<Self::Item> {
        let prev_next = self.cursor.prev.load(Ordering::Relaxed, self.guard);
        // 当list更新后，需要跟cursor比较，如果不同就中断，然后重新获取迭代器迭代
        if self.cursor.curr != prev_next {
            return Some(Err(()));
        }
        let current = unsafe { self.cursor.curr.as_ref()? };
        let mut next = unsafe { current.next.read_lock() };

        self.cursor.curr = next.load(Ordering::Relaxed, self.guard);
        mem::swap(&mut self.cursor.prev, &mut next);

        next.finish();
        Some(Ok(&current.data))
    }
}

impl<T: std::fmt::Display> Drop for OptimisticFineGrainedListSet<T> {
    fn drop(&mut self) {
        let mut o_curr = mem::replace(&mut self.head, SeqLock::new(Atomic::null()));
        while let Some(curr) = unsafe { o_curr.into_inner().try_into_owned() }.map(Owned::into_box)
        {
            o_curr = curr.next;
        }
    }
}

impl<T: std::fmt::Display> Default for OptimisticFineGrainedListSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

