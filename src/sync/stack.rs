use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

pub struct Stack<T> {
    mu: Mutex<Vec<T>>,
    cv: Condvar,
    len: AtomicUsize,
}

impl<T> Default for Stack<T> {
    fn default() -> Self {
        Self {
            mu: Mutex::new(vec![]),
            cv: Condvar::new(),
            len: AtomicUsize::new(0),
        }
    }
}

impl<T: 'static + Send> Stack<T> {
    pub fn push(&self, item: T) {
        let mut mu = self.mu.lock().unwrap();
        mu.push(item);
        //self.len.fetch_add(1, Ordering::Release);
        drop(mu);
        //self.cv.notify_all();
    }

    pub fn pop(&self) -> T {
        let mut mu = self.mu.lock().unwrap();
        while mu.is_empty() {
            mu = self.cv.wait(mu).unwrap();
        }

        self.len.fetch_sub(1, Ordering::Release);

        mu.pop().unwrap()
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    pub fn try_pop(&self) -> Option<T> {
        /*
        if self.len.load(Ordering::Acquire) == 0 {
            return None;
        }
        */

        let mut mu = self.mu.lock().unwrap();

        let ret = mu.pop();

        if ret.is_some() {
            self.len.fetch_sub(1, Ordering::Release);
        }

        ret
    }
}

#[test]
fn smoke_stack() {
    const WRITE_THREADS: usize = 16;
    const READ_THREADS: usize = 16;
    const N_PER_THREAD: usize = 1_000_000;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread::spawn;

    use rand::{Rng, rng};

    let mut threads = vec![];

    let barrier = Arc::new(Barrier::new(WRITE_THREADS + READ_THREADS));
    let stack = Arc::new(Stack::default());
    let read_count = Arc::new(AtomicUsize::new(0));

    for _ in 0..WRITE_THREADS {
        let barrier = barrier.clone();
        let stack = stack.clone();

        let thread = spawn(move || {
            barrier.wait();

            let mut rng = rng();

            for _ in 0..N_PER_THREAD {
                let value: u64 = rng.random();
                stack.push(value);
            }
        });

        threads.push(thread);
    }

    for _ in 0..READ_THREADS {
        let barrier = barrier.clone();
        let stack = stack.clone();
        let read_count = read_count.clone();

        let thread = spawn(move || {
            barrier.wait();

            while read_count.load(Ordering::Acquire)
                < WRITE_THREADS * N_PER_THREAD
            {
                if stack.try_pop().is_some() {
                    read_count.fetch_add(1, Ordering::Release);
                }
            }
        });

        threads.push(thread);
    }

    for thread in threads.into_iter() {
        thread.join().unwrap();
    }
}

#[test]
fn smoke_stack_mut_vec() {
    const WRITE_THREADS: usize = 16;
    const READ_THREADS: usize = 16;
    const N_PER_THREAD: usize = 1_000_000;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread::spawn;

    use rand::{Rng, rng};

    let mut threads = vec![];

    let barrier = Arc::new(Barrier::new(WRITE_THREADS + READ_THREADS));
    let stack = Arc::new(Mutex::new(Vec::default()));
    let read_count = Arc::new(AtomicUsize::new(0));

    for _ in 0..WRITE_THREADS {
        let barrier = barrier.clone();
        let stack = stack.clone();

        let thread = spawn(move || {
            barrier.wait();

            let mut rng = rng();

            for _ in 0..N_PER_THREAD {
                let value: u64 = rng.random();
                let mut mu = stack.lock().unwrap();
                mu.push(value);
            }
        });

        threads.push(thread);
    }

    for _ in 0..READ_THREADS {
        let barrier = barrier.clone();
        let stack = stack.clone();
        let read_count = read_count.clone();

        let thread = spawn(move || {
            barrier.wait();

            while read_count.load(Ordering::Acquire)
                < WRITE_THREADS * N_PER_THREAD
            {
                let mut mu = stack.lock().unwrap();
                if mu.pop().is_some() {
                    read_count.fetch_add(1, Ordering::Release);
                }
            }
        });

        threads.push(thread);
    }

    for thread in threads.into_iter() {
        thread.join().unwrap();
    }
}
