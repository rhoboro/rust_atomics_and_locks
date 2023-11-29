use crate::oneshot_channel_nonblocking::Channel;
use std::thread;

//
// mod oneshot_channel;
// mod oneshot_channel_arc;
// mod oneshot_channel_lifetime;
mod oneshot_channel_nonblocking;
// mod simple_channel;

fn main() {
    println!("Hello, world!");
    let mut channel = Channel::new();
    thread::scope(|s| {
        let (sender, receiver) = channel.split();
        s.spawn(move || {
            sender.send("hello world!!");
        });
        assert_eq!(receiver.receive(), "hello world!!");
    });
}
