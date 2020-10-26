#[macro_use]
extern crate criterion;

use chan_thread::{FutureThread, FutureThread2};
use criterion::black_box;
use criterion::Criterion;
use futures::executor::block_on;

fn future_add(c: &mut Criterion) {
    c.bench_function("future add", |b| {
        let fut_thread = FutureThread::new(|(a, b)| a * a + b * b);
        b.iter(move || block_on(fut_thread.call((black_box(1), black_box(2)))))
    });
}

fn old_add(c: &mut Criterion) {
    use fut_old::{future::ok, Future};
    use fut_old_spawn::SpawnHelper;
    use fut_old_threadpool::ThreadPool;

    c.bench_function("future old add", |b| {
        let pool = ThreadPool::new(1);
        let x: u32 = black_box(1);
        let y: u32 = black_box(2);
        b.iter(move || pool.spawn(ok::<_, ()>(x + y)).wait().unwrap());
    });
}

fn future2_add(c: &mut Criterion) {
    c.bench_function("future2 add", |b| {
        let x = black_box(1);
        let y = black_box(2);
        let mut fut_thread = FutureThread2::new();
        b.iter(move || block_on(fut_thread.spawn(move || x * x + y * y)))
    });
}

criterion_group!(benches, future_add, old_add, future2_add);
criterion_main!(benches);
