use futures::{
    StreamExt,
    channel::mpsc,
    executor::{self, ThreadPool},
};

fn main() {
    let pool = ThreadPool::new().unwrap();
    let (tx, rx) = mpsc::unbounded::<i32>();

    let fut_values = async {
        let fut_tx_result = async move { (0..10).for_each(|num| tx.unbounded_send(num).unwrap()) };
        pool.spawn_ok(fut_tx_result);

        let fut_values = rx.map(|num| num * 2).collect::<Vec<i32>>();
        fut_values.await
    };

    let values = executor::block_on(fut_values);
    println!("values {:?}", values);
}
