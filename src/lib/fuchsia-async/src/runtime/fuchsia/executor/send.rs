// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use super::{
    common::{
        next_deadline, with_local_timer_heap, EHandle, ExecutorTime, Inner, TimerHeap,
        EMPTY_WAKEUP_ID, TASK_READY_WAKEUP_ID,
    },
    time::Time,
};
use crate::runtime::fuchsia::executor::instrumentation::WakeupReason;
use fuchsia_zircon::{self as zx};
use futures::{
    future::{self, FutureObj},
    FutureExt,
};
use parking_lot::{Condvar, Mutex};
use std::{
    fmt,
    future::Future,
    mem,
    sync::{atomic::Ordering, Arc},
    thread, usize,
};

/// A multi-threaded port-based executor for Fuchsia OS. Requires that tasks scheduled on it
/// implement `Send` so they can be load balanced between worker threads.
///
/// Having a `SendExecutor` in scope allows the creation and polling of zircon objects, such as
/// [`fuchsia_async::Channel`].
///
/// # Panics
///
/// `SendExecutor` will panic on drop if any zircon objects attached to it are still alive. In other
/// words, zircon objects backed by a `SendExecutor` must be dropped before it.
pub struct SendExecutor {
    /// The inner executor state.
    inner: Arc<Inner>,
}

impl fmt::Debug for SendExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalExecutor").field("port", &self.inner.port).finish()
    }
}

impl SendExecutor {
    /// Create a new multi-threaded executor.
    // TODO(fxbug.dev/76550) move the number of threads here
    pub fn new() -> Result<Self, zx::Status> {
        let inner = Arc::new(Inner::new(ExecutorTime::RealTime)?);
        inner.clone().set_local(TimerHeap::new());
        Ok(Self { inner })
    }

    /// Run `future` to completion, using this thread and `num_threads - 1` workers in a pool to
    /// poll active tasks.
    pub fn run<F>(&mut self, future: F, num_threads: usize) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.inner.require_real_time().expect("Error: called `run` on an executor using fake time");
        self.inner.threadiness.require_multithreaded().expect(
            "Error: called `run` on executor after using `spawn_local`. \
             Use `run_singlethreaded` instead.",
        );

        let pair = Arc::new((Mutex::new(None), Condvar::new()));
        let pair2 = pair.clone();

        // Spawn a future which will set the result upon completion.
        Inner::spawn(
            &self.inner,
            FutureObj::new(Box::new(future.then(move |fut_result| {
                let (lock, cvar) = &*pair2;
                let mut result = lock.lock();
                *result = Some(fut_result);
                cvar.notify_one();
                future::ready(())
            }))),
        );

        // Start worker threads, handing off timers from the current thread.
        self.inner.done.store(false, Ordering::SeqCst);
        with_local_timer_heap(|timer_heap| {
            let timer_heap = mem::replace(timer_heap, TimerHeap::new());
            self.create_worker_threads(num_threads, Some(timer_heap));
        });

        // Wait until the signal the future has completed.
        let (lock, cvar) = &*pair;
        let mut result = lock.lock();
        while result.is_none() {
            cvar.wait(&mut result);
        }

        // Spin down worker threads
        self.inner.done.store(true, Ordering::SeqCst);
        self.join_all();

        // Unwrap is fine because of the check to `is_none` above.
        result.take().unwrap()
    }

    /// Add `num_workers` worker threads to the executor's thread pool.
    /// `timers`: timers from the "master" thread which would otherwise be lost.
    fn create_worker_threads(&self, num_workers: usize, mut timers: Option<TimerHeap>) {
        let mut threads = self.inner.threads.lock();
        for _ in 0..num_workers {
            threads.push(self.new_worker(timers.take()));
        }
    }

    fn join_all(&self) {
        let mut threads = self.inner.threads.lock();

        // Send a user packet to wake up all the threads
        for _thread in threads.iter() {
            self.inner.notify_empty();
        }

        // Join the worker threads
        for thread in threads.drain(..) {
            thread.join().expect("Couldn't join worker thread.");
        }
    }

    fn new_worker(&self, timers: Option<TimerHeap>) -> thread::JoinHandle<()> {
        let inner = self.inner.clone();
        thread::spawn(move || Self::worker_lifecycle(inner, timers))
    }

    fn worker_lifecycle(inner: Arc<Inner>, timers: Option<TimerHeap>) {
        inner.clone().set_local(timers.unwrap_or(TimerHeap::new()));
        let mut local_collector = inner.collector.create_local_collector();
        loop {
            if inner.done.load(Ordering::SeqCst) {
                EHandle::rm_local();
                return;
            }

            let packet = with_local_timer_heap(|timer_heap| {
                let deadline = next_deadline(timer_heap).map(|t| t.time).unwrap_or(Time::INFINITE);

                local_collector.will_wait();
                // into_zx: we are using real time, so the time is a monotonic time.
                match inner.port.wait(deadline.into_zx()) {
                    Ok(packet) => Some(packet),
                    Err(zx::Status::TIMED_OUT) => {
                        local_collector.woke_up(WakeupReason::Deadline);
                        let time_waker = timer_heap.pop().unwrap();
                        time_waker.wake();
                        None
                    }
                    Err(status) => {
                        panic!("Error calling port wait: {:?}", status);
                    }
                }
            });

            if let Some(packet) = packet {
                match packet.key() {
                    EMPTY_WAKEUP_ID => local_collector.woke_up(WakeupReason::Notification),
                    TASK_READY_WAKEUP_ID => {
                        local_collector.woke_up(WakeupReason::Notification);
                        inner.poll_ready_tasks(&mut local_collector);
                    }
                    receiver_key => {
                        local_collector.woke_up(WakeupReason::Io);
                        inner.deliver_packet(receiver_key as usize, packet);
                    }
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> super::instrumentation::Snapshot {
        self.inner.collector.snapshot()
    }
}

impl Drop for SendExecutor {
    fn drop(&mut self) {
        self.inner.mark_done();
        // Wake the threads so they can kill themselves.
        self.join_all();
        self.inner.on_parent_drop();
    }
}

// TODO(fxbug.dev/76583) test SendExecutor with unit tests
