#![warn(clippy::missing_errors_doc, clippy::result_large_err)]

use std::cell::UnsafeCell;
use std::hint::spin_loop;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

struct MyArcInner<T> {
    count: AtomicUsize,
    value: T,
}

pub struct MyArc<T> {
    ptr: NonNull<MyArcInner<T>>,
}

unsafe impl<T: Send + Sync> Send for MyArc<T> {}
unsafe impl<T: Send + Sync> Sync for MyArc<T> {}

impl<T> MyArc<T> {
    pub fn new(value: T) -> Self {
        let inner = Box::new(MyArcInner {
            count: AtomicUsize::new(1),
            value,
        });

        Self {
            ptr: NonNull::new(Box::into_raw(inner)).expect("Box pointer cannot be null"),
        }
    }

    fn inner(&self) -> &MyArcInner<T> {
        unsafe { self.ptr.as_ref() }
    }

    pub fn strong_count(&self) -> usize {
        self.inner().count.load(Ordering::Acquire)
    }
}

impl<T> Clone for MyArc<T> {
    fn clone(&self) -> Self {
        self.inner().count.fetch_add(1, Ordering::Relaxed);

        Self { ptr: self.ptr }
    }
}

impl<T> Deref for MyArc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner().value
    }
}

impl<T> Drop for MyArc<T> {
    fn drop(&mut self) {
        if self.inner().count.fetch_sub(1, Ordering::Release) == 1 {
            std::sync::atomic::fence(Ordering::Acquire);
            unsafe {
                drop(Box::from_raw(self.ptr.as_ptr()));
            }
        }
    }
}

pub struct MyMutex<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for MyMutex<T> {}
unsafe impl<T: Send> Sync for MyMutex<T> {}

pub struct MyMutexGuard<'a, T> {
    mutex: &'a MyMutex<T>,
}

impl<T> MyMutex<T> {
    pub fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> MyMutexGuard<'_, T> {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            spin_loop();
        }

        MyMutexGuard { mutex: self }
    }
}

impl<T> Deref for MyMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> DerefMut for MyMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for MyMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}

fn benchmark_std_arc_mutex(threads: usize, iterations: usize) -> u128 {
    let counter = Arc::new(Mutex::new(0usize));

    let start = Instant::now();

    let mut handles = Vec::new();

    for _ in 0..threads {
        let counter = Arc::clone(&counter);

        handles.push(thread::spawn(move || {
            for _ in 0..iterations {
                let mut value = counter.lock().expect("Mutex poisoned");
                *value += 1;
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    start.elapsed().as_millis()
}

fn benchmark_my_arc_mutex(threads: usize, iterations: usize) -> u128 {
    let counter = MyArc::new(MyMutex::new(0usize));

    let start = Instant::now();

    let mut handles = Vec::new();

    for _ in 0..threads {
        let counter = counter.clone();

        handles.push(thread::spawn(move || {
            for _ in 0..iterations {
                let mut value = counter.lock();
                *value += 1;
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    start.elapsed().as_millis()
}

fn print_result(std_time: u128, my_time: u128) {
    println!("std Arc + Mutex: {std_time} ms");
    println!("MyArc + MyMutex: {my_time} ms");

    if std_time == 0 {
        println!("Cannot calculate percentage because std time is 0 ms");
        return;
    }

    if my_time > std_time {
        let diff = ((my_time - std_time) as f64 / std_time as f64) * 100.0;
        println!("My implementation is slower by {diff:.2}%");
    } else {
        let diff = ((std_time - my_time) as f64 / std_time as f64) * 100.0;
        println!("My implementation is faster by {diff:.2}%");
    }
}

fn main() {
    let threads = 4;
    let iterations = 100;

    println!("Threads: {threads}");
    println!("Iterations per thread: {iterations}");

    let std_time = benchmark_std_arc_mutex(threads, iterations);
    let my_time = benchmark_my_arc_mutex(threads, iterations);

    print_result(std_time, my_time);
}