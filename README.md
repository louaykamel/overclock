

<h3 align="center">Actor framework for building data driven distributed systems</h2>

<p align="center">
    <a href="https://github.com/louaykamel/overclock/actions" style="text-decoration:none;"><img src="https://github.com/louaykamel/overclock/workflows/Build/badge.svg"></a>
    <a href="https://github.com/louaykamel/overclock/actions" style="text-decoration:none;"><img src="https://github.com/louaykamel/overclock/workflows/Test/badge.svg"></a>
    <a href="https://github.com/louaykamel/overclock/blob/master/LICENSE" style="text-decoration:none;"><img src="https://img.shields.io/badge/License-Apache%202.0-green.svg" alt="Apache 2.0 license"></a>
    <a href="https://dependabot.com" style="text-decoration:none;"><img src="https://api.dependabot.com/badges/status?host=github&repo=louaykamel/overclock" alt=""></a>
</p>

---

# About

Overclock is an actor model framework inspired by Elixir, enforces supervision tree and interoperability.

# Features

* Async
* Based on Tokio
* Multiple channel types
* Actor local store, accessible through directory path interface
* Websocket server for RPC communication
* Built-in config support
* Dynamic Topology
* Reusable actors
* Promethues supports
* Communication

# Usage

Add overclock to your Cargo.toml:
```toml
[dependencies]
overclock = "0.1"
```

## Implement Actor trait
```rust

use overclock::core::*;

// Define your actor struct
#[derive(Debug, Serialize, Deserialize, PartialEq, Default)]
struct HelloWorld;

#[async_trait::async_trait]
impl<S> Actor<S> for HelloWorld
where
    S: SupHandle<Self>,
{
    // Temporary state or resources during the actor lifecycle
    type Data = usize;
    // Interval channel which will yield Instant every 1000ms;
    type Channel = IntervalChannel<1000>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        log::info!("HelloWorld: {}", rt.service().status());
        let counter = 0;
        Ok(counter)
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, mut counter: Self::Data) -> ActorResult<()> {
        log::info!("HelloWorld: {}", rt.service().status());
        while let Some(event) = rt.inbox_mut().next().await {
            if counter == 3 {
                counter += 1;
                log::info!("HelloWorld: Received instant {:?}, counter: {}", event, counter);
            } else {
                break
            }
        }
        log::info!("HelloWorld: exited its event loop");
        Ok(())
    }
}


#[tokio::main]
async fn main() {
    let runtime = Runtime::from_config::<HelloWorld>().await.expect("Runtime to run");
    runtime.block_on().await.expect("Runtime to shutdown gracefully");
}

```
## Run the above illustrated example

```shel
cargo run --features="ron_config"
```
## Contributing

All contributions are welcome!

## LICENSE

This project is licensed under either of

* Apache License, Version 2.0, (LICENSE-APACHE or https://www.apache.org/licenses/LICENSE-2.0)

## COPYRIGHT

    Copyright (C) 2022 Louay Kamel
    Copyright (C) 2021 IOTA Stiftung
