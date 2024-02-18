//! Testing utilities for set types

use core::fmt::Debug;
use core::hash::Hash;
use rand::prelude::*;
use std::collections::{HashMap, HashSet};
use std::thread;

use crate::test::RandGen;
use crate::ConcurrentSet;

/// Runs many operations in a single thread and tests if it works like a set data structure using
/// `std::collections::HashSet` as reference.
pub fn stress_sequential<K: Debug + Clone + Eq + Hash + RandGen, S: Default + ConcurrentSet<K>>(
    steps: usize,
) {
    #[derive(Debug)]
    enum Ops {
        ContainsSome,
        ContainsNone,
        Insert,
        RemoveSome,
        RemoveNone,
    }

    let ops = [
        Ops::ContainsSome,
        Ops::ContainsNone,
        Ops::Insert,
        Ops::RemoveSome,
        Ops::RemoveNone,
    ];
    let mut rng = thread_rng();
    let set = S::default();
    let mut hashset = HashSet::<K>::new();

    for i in 0..steps {
        let op = ops.choose(&mut rng).unwrap();

        match op {
            Ops::ContainsSome => {
                if let Some(key) = hashset.iter().choose(&mut rng) {
                    println!("iteration {i}: contains({key:?}) (existing)");
                    assert_eq!(set.contains(key), hashset.contains(key));
                }
            }
            Ops::ContainsNone => {
                let key = K::rand_gen(&mut rng);
                println!("iteration {i}: contains({key:?}) (non-existing)");
                assert_eq!(set.contains(&key), hashset.contains(&key));
            }
            Ops::Insert => {
                let key = K::rand_gen(&mut rng);
                println!("iteration {i}: insert({key:?})");
                assert_eq!(set.insert(key.clone()), hashset.insert(key));
            }
            Ops::RemoveSome => {
                let key = hashset.iter().choose(&mut rng).map(Clone::clone);
                if let Some(key) = key {
                    println!("iteration {i}: remove({key:?}) (existing)");
                    assert_eq!(set.remove(&key), hashset.remove(&key));
                }
            }
            Ops::RemoveNone => {
                let key = K::rand_gen(&mut rng);
                println!("iteration {i}: remove({key:?}) (non-existing)");
                assert_eq!(set.remove(&key), hashset.remove(&key));
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Ops {
    Contains,
    Insert,
    Remove,
}

#[derive(Debug, Clone)]
enum Log<K> {
    Contains { key: K, result: bool },
    Insert { key: K, result: bool },
    Remove { key: K, result: bool },
}

impl<K> Log<K> {
    fn key(&self) -> &K {
        match self {
            Self::Contains { key, .. } => key,
            Self::Insert { key, .. } => key,
            Self::Remove { key, .. } => key,
        }
    }
}

/// Randomly runs many operations concurrently.
pub fn stress_concurrent<K: Debug + Clone + Eq + RandGen, S: Default + Sync + ConcurrentSet<K>>(
    threads: usize,
    steps: usize,
) {
    let ops = [Ops::Contains, Ops::Insert, Ops::Remove];

    let set = S::default();

    thread::scope(|s| {
        for _ in 0..threads {
            let _ununsed = s.spawn(|| {
                let mut rng = thread_rng();
                for _ in 0..steps {
                    let op = ops.choose(&mut rng).unwrap();

                    match op {
                        Ops::Contains => {
                            let value = K::rand_gen(&mut rng);
                            let _ = set.contains(&value);
                            let value = &value;
                            println!("contains({value:?}) (existing)");
                        }

                        Ops::Insert => {
                            let value = K::rand_gen(&mut rng);
                            let tt = &value;
                            println!("insert({tt:?})");
                            let _unused = set.insert(value);

                        }

                        Ops::Remove => {
                            let value = K::rand_gen(&mut rng);
                            let _unused = set.remove(&value);
                            let value = &value;
                            println!("remove({value:?})");
                        }
                        _ => ()
                    }
                }
            });
        }
    });
}

fn assert_logs_consistent<K: Clone + Eq + Hash>(logs: &Vec<Vec<Log<K>>>) {
    let mut per_key_logs = HashMap::<K, Vec<Log<K>>>::new();
    for ls in logs {
        for l in ls {
            per_key_logs
                .entry(l.key().clone())
                .or_default()
                .push(l.clone());
        }
    }

    for (k, logs) in &per_key_logs {
        let mut inserts = HashMap::<K, usize>::new();
        let mut deletes = HashMap::<K, usize>::new();

        for l in logs {
            match l {
                Log::Insert { result: true, .. } => *inserts.entry(k.clone()).or_insert(0) += 1,
                Log::Remove { result: true, .. } => *deletes.entry(k.clone()).or_insert(0) += 1,
                _ => (),
            }
        }

        for l in logs {
            if let Log::Contains { key, result: true } = l {
                assert!(inserts.contains_key(key))
            }
        }

        for (k, v) in &deletes {
            assert!(inserts.get(k).unwrap() >= v);
        }
    }
}

/// Randomly runs many operations concurrently and logs the operations & results per thread. Then
/// checks the consistency of the log. For example, if the key `k` was successfully deleted twice,
/// then `k` must have been inserted at least twice.
pub fn log_concurrent<
    K: Debug + Clone + Eq + Hash + Send + RandGen,
    S: Default + Sync + ConcurrentSet<K>,
>(
    threads: usize,
    steps: usize,
) {
    let ops = [Ops::Contains, Ops::Insert, Ops::Remove];

    let set = S::default();

    let logs = thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..threads {
            let handle = s.spawn(|| {
                let mut rng = thread_rng();
                let mut logs = Vec::new();
                for _ in 0..steps {
                    let op = ops.choose(&mut rng).unwrap();

                    match op {
                        Ops::Contains => {
                            let key = K::rand_gen(&mut rng);
                            let result = set.contains(&key);
                            logs.push(Log::Contains {
                                key: key.clone(),
                                result,
                            });
                        }
                        Ops::Insert => {
                            let key = K::rand_gen(&mut rng);
                            let result = set.insert(key.clone());
                            logs.push(Log::Insert { key, result });
                        }
                        Ops::Remove => {
                            let key = K::rand_gen(&mut rng);
                            let result = set.remove(&key);
                            logs.push(Log::Remove {
                                key: key.clone(),
                                result,
                            });
                        }
                    }
                }
                logs
            });
            handles.push(handle);
        }
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });

    assert_logs_consistent(&logs);
}
