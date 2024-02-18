use std::cmp;
use std::mem;
use std::ops::Deref;
use std::ptr;
use std::sync::{Mutex, MutexGuard};
use std::io::{self, Write};

use crate::ConcurrentSet;

#[derive(Debug)]
struct Node<T> {
    data: T,
    next: Mutex<*mut Node<T>>,
}

/// Concurrent sorted singly linked list using fine-grained lock-coupling.
#[derive(Debug)]
pub struct FineGrainedListSet<T> {
    head: Mutex<*mut Node<T>>,
}

unsafe impl<T: Send> Send for FineGrainedListSet<T> {}
unsafe impl<T: Send> Sync for FineGrainedListSet<T> {}

// reference to the `next` field of previous node which points to the current node
//  pre -> node
struct Cursor<'l, T>(MutexGuard<'l, *mut Node<T>>);

impl<T> Node<T> {
    fn new(data: T, next: *mut Self) -> *mut Self {
        Box::into_raw(Box::new(Self {
            data,
            next: Mutex::new(next),
        }))
    }
}

// find
impl<T: Ord> Cursor<'_, T> {
    /// Moves the cursor to the position of key in the sorted list.
    /// Returns whether the value was found.
    ///
    // list a b c d
    // cursor(b)
    //


    fn find(&mut self, key: &T) -> bool {
        return true;
        // todo!()
        // unsafe {
        //     // let mut head = self.head.lock().unwrap();
        //     let mut new_node = Node::new(key, ptr::null_mut());
        //     let mut head = self.0;
        //     mem::swap()
        //     loop {
        //         if head.is_null() {
        //             return false;
        //         }
        //         if (**head).data.eq(key) {
        //             return true;
        //         } else {
        //             head = (**head).next.lock().unwrap();
        //         }
        //     }
        // }
    }
}

impl<T> FineGrainedListSet<T> {
    /// Creates a new list.
    pub fn new() -> Self {
        Self {
            head: Mutex::new(ptr::null_mut()),
        }
    }
}

impl<T: Ord> FineGrainedListSet<T> {
    fn find(&self, key: &T) -> (bool, Cursor<'_, T>) {
        // todo!()
        // head
        unsafe {
            let mut head = self.head.lock().unwrap();
            loop {
                if head.is_null() {
                    return (false, Cursor(head));
                }
                if (**head).data.eq(key) {
                    return (true, Cursor(head));
                } else {
                    head = (**head).next.lock().unwrap();
                }
            }
        }
    }
}

impl<T: Ord> ConcurrentSet<T> for FineGrainedListSet<T> {
    fn contains(&self, key: &T) -> bool {
        self.find(key).0
    }

    // 只先在head的地方插入
    // insert remove
    // remove insert

    fn insert(&self, key: T) -> bool {

        // todo!();

        if self.contains(&key) {
            return false;
        }
        // 不包含 key
        //

        let mut head = self.head.lock().unwrap();
        if head.is_null() {
            *head = Node::new(key, ptr::null_mut());
            return true;
        }

        loop {
            let mut head_pointer = unsafe {&**head};
            // 目前有两种情况，一种是 head_pointer < key, 一种是head_pointer > key
            // 这里的head_pointer 值得是curr_node， 该
            if head_pointer.data.lt(&key) {
                let mut head_pointer_next_guard = head_pointer.next.lock().unwrap();
                if head_pointer_next_guard.is_null() {
                    *head_pointer_next_guard = Node::new(key, ptr::null_mut());
                    return true;
                }
                head = head_pointer_next_guard;
                head_pointer = unsafe {&**head};
            }
            else {
                let head_copy = *head;
                let new_node = Node::new(key, head_copy);
                // new_node -> next = head
                // head = new_node
                unsafe {
                    *head = new_node;
                }
                return true;
            }
        }
    }
    // remove (contain(true), _remove)

    fn remove(&self, key: &T) -> bool {
        // todo!()

        if !self.contains(&key) {
            return false;
        }

        // remove

        // 这里要考虑到的一个情况是 insert(x) remove(x) remove(x)
        let mut head = self.head.lock().unwrap();

        while !head.is_null() {
            let current_node = unsafe {&**head};
            if current_node.data.eq(key) {
                break;
            }
            head = current_node.next.lock().unwrap();
        }

        // 在找要删除的点的时候要再确认一下是否已经被删除过了
        if head.is_null() {
            return false;
        }
        // 释放被删除节点内存
        let cur_node = unsafe {Box::from_raw(*head)};
        // head = head -> next
        *head = *cur_node.next.lock().unwrap();
        return true;

    }
}

#[derive(Debug)]
pub struct Iter<'l, T>(MutexGuard<'l, *mut Node<T>>);

impl<T> FineGrainedListSet<T> {
    /// An iterator visiting all elements.
    pub fn iter(&self) -> Iter<'_, T> {
        Iter(self.head.lock().unwrap())
    }
}

impl<'l, T> Iterator for Iter<'l, T> {
    type Item = &'l T;

    fn next(&mut self) -> Option<Self::Item> {
        // todo!()
        unsafe {
            if (*self.0).is_null() {
                return None;
            } else {
                let mut tt = &(*(*(self.0))).data;
                self.0 = (*(*self.0)).next.lock().unwrap();
                Some(tt)
            }
        }
    }
}

impl<T> Drop for FineGrainedListSet<T> {
    fn drop(&mut self) {
        // todo!()

        let mut head = *self.head.lock().unwrap();
        while !head.is_null() {
            let tmp = unsafe { Box::from_raw(head)  };
            head = *tmp.next.lock().unwrap();
        }
    }
}

impl<T> Default for FineGrainedListSet<T> {
    fn default() -> Self {
        Self::new()
    }
}
