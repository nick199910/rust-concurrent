use rand::prelude::*;
use std::collections::HashSet;
use std::iter::zip;
use std::sync::atomic::{
    AtomicBool,
    Ordering::{Acquire, Release},
};
use std::thread;
use crate::{ConcurrentSet, FineGrainedListSet};
use crate::test::adt::set;

// #[test]
fn smoke() {
    let set = FineGrainedListSet::new();
    assert!(set.insert(1));

    assert!(set.insert(2));

    assert!(set.insert(3));

    // assert!(set.remove(&2));
    // for (r, v) in zip(set.iter(), [1, 3]) {
    //     assert_eq!(*r, v);
    // }
    // assert!(set.remove(&3));
}

// #[test]
fn my_test() {
    let set = FineGrainedListSet::new();
    assert_eq!(set.remove(&222), false);

    assert_eq!(set.insert(184), true);
    assert_eq!(set.insert(15), true);
    assert_eq!(set.insert(182), true);

    for i in set.iter() {
        println!("{}", *i);
    }
    // assert_eq!(set.contains(&182), true);

}

// #[test]
fn stress_sequential() {
    const STEPS: usize = 4096;
    set::stress_sequential::<u8, FineGrainedListSet<u8>>(STEPS);


}

// #[test]
fn stress_concurrent() {
    const THREADS: usize = 16;
    const STEPS: usize = 4096 * 16;
    // const THREADS: usize = 8;
    // const STEPS: usize = 16;
    set::stress_concurrent::<u8, FineGrainedListSet<u8>>(THREADS, STEPS);

}

// #[test]
fn log_concurrent() {
    const THREADS: usize = 16;
    const STEPS: usize = 4096 * 16;
    set::log_concurrent::<u8, FineGrainedListSet<u8>>(THREADS, STEPS);
}

/// Check the consistency of iterator while other operations are running concurrently.
// #[test]
fn iter_consistent() {
    const THREADS: usize = 15;
    const STEPS: usize = 4096 * 16;


    let set = FineGrainedListSet::new();

    // pre-fill with even numbers
    for i in (0..100).step_by(2).rev() {
        assert!(set.insert(i));
    }
    let evens = set.iter().copied().collect::<HashSet<_>>();

    let done = AtomicBool::new(false);
    thread::scope(|s| {
        // insert or remove odd numbers
        for _ in 0..THREADS {
            let _unused = s.spawn(|| {
                let mut rng = thread_rng();
                for _ in 0..STEPS {
                    let key = 2 * rng.gen_range(0..50) + 1;
                    if rng.gen() {
                        set.insert(key);
                    } else {
                        set.remove(&key);
                    }
                }
                done.store(true, Release);
            });
        }
        let _unused = s.spawn(|| {
            while !done.load(Acquire) {
                let snapshot = set.iter().copied().collect::<Vec<_>>();
                // sorted
                assert!(snapshot.windows(2).all(|k| k[0] <= k[1]));
                // even numbers are not touched
                let snapshot = snapshot.into_iter().collect::<HashSet<_>>();
                assert!(evens.is_subset(&snapshot));
            }
        });
    });
}
