use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

use ebr::Ebr;

pub struct Queue<T: 'static + Send> {
    head: Arc<AtomicPtr<Node<T>>>,
    tail: Arc<AtomicPtr<Node<T>>>,
    ebr: Ebr<Box<Node<T>>>,
}

impl<T: 'static + Send> Default for Queue<T> {
    fn default() -> Self {
        // we construct a queue where there is one sentinel node that both head and tail point to
        let sentinel_node = Box::new(Node {
            item: None,
            next: AtomicPtr::default(),
            is_sentinel: true,
        });

        let sentinel_ptr = Box::into_raw(sentinel_node);

        Self {
            head: Arc::new(AtomicPtr::new(sentinel_ptr)),
            tail: Arc::new(AtomicPtr::new(sentinel_ptr)),
            ebr: Ebr::default(),
        }
    }
}

struct Node<T> {
    item: Option<T>,
    next: AtomicPtr<Node<T>>,
    is_sentinel: bool,
}

impl<T: 'static + Send> Queue<T> {
    pub fn push(&self, item: T) {
        let mut tail = self.tail.load(Ordering::Acquire);

        let mut node = Box::new(Node {
            item: Some(item),
            next: AtomicPtr::default(),
            is_sentinel: false,
        });

        loop {
            let node_ptr = Box::into_raw(node);

            match self.tail.compare_exchange_weak(
                tail,
                node_ptr,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return;
                }
                Err(more_recent_head) => {
                    next = more_recent_head;
                    node = unsafe { Box::from_raw(node_ptr) };
                    node.next = next;
                }
            }
        }
    }

    pub fn try_pop(&self) -> Option<T> {
        let mut head = self.head.load(Ordering::Acquire);
        let mut guard = self.ebr.pin();
        loop {
            // invariant: head is never null due to sentinel node
            assert!(!head.is_null());

            let head_ref = unsafe { &*head };
            if head_ref.is_sentinel {
                return None;
            }

            let next: *mut Node<T> = head_ref.next.load(Ordering::Acquire);

            match self.head.compare_exchange_weak(
                head,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let mut node = unsafe { Box::from_raw(head) };
                    let item = node.item.take().unwrap();
                    guard.defer_drop(node);
                    return Some(item);
                }
                Err(more_recent_head) => {
                    head = more_recent_head;
                }
            }
        }
    }
}

#[test]
fn smoke_queue() {
    const WRITE_THREADS: usize = 16;
    const READ_THREADS: usize = 16;
    const N_PER_THREAD: usize = 1_000_000;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread::spawn;

    use rand::{Rng, rng};

    let mut threads = vec![];

    let barrier = Arc::new(Barrier::new(WRITE_THREADS + READ_THREADS));
    let queue = Queue::default();
    let read_count = Arc::new(AtomicUsize::new(0));

    for _ in 0..WRITE_THREADS {
        let barrier = barrier.clone();
        let queue = queue.clone();

        let thread = spawn(move || {
            barrier.wait();

            let mut rng = rng();

            for _ in 0..N_PER_THREAD {
                let value: u64 = rng.random();
                queue.push(value);
            }
        });

        threads.push(thread);
    }

    for _ in 0..READ_THREADS {
        let barrier = barrier.clone();
        let queue = queue.clone();
        let read_count = read_count.clone();

        let thread = spawn(move || {
            barrier.wait();

            while read_count.load(Ordering::Acquire)
                < WRITE_THREADS * N_PER_THREAD
            {
                if let Some(popped) = queue.try_pop() {
                    read_count.fetch_add(1, Ordering::Release);
                }
            }
        });
    }

    for thread in threads.into_iter() {
        thread.join().unwrap();
    }
}
