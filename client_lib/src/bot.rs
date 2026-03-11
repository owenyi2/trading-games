use std::net::TcpStream;
use std::sync::{atomic, mpsc};
use std::thread::{self, JoinHandle};
use std::time;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use client_lib::Client;
use exchange::types::*;

use tungstenite;
use tungstenite::stream::MaybeTlsStream;

struct MarketMaker {
    client: Client,
    theo: u64,
    last_change: Instant,
}

impl MarketMaker {
    fn new(client: Client) -> Self {
        Self {
            client,
            theo: 100,
            last_change: Instant::now(),
        }
    }
    fn act(&mut self) {
        let now = Instant::now();
        let EXCHANGE_ID = 0;

        if now.duration_since(self.last_change) >= Duration::from_secs(1) {
            self.last_change = now;

            // simple coin flip without rand
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .subsec_nanos();

            if (nanos >> 10) % 2 == 0 {
                self.theo += 1;
            } else {
                self.theo -= 1;
            }

            self.client.cancel_all(EXCHANGE_ID);

            let qty = 10;
            let bid_price = self.theo - 1;
            let ask_price = self.theo + 1;

            self.client
                .limit_order(EXCHANGE_ID, Side::Bid, bid_price, qty);
            self.client
                .limit_order(EXCHANGE_ID, Side::Ask, ask_price, qty);
        }
    }
    fn run(&mut self) {
        loop {
            self.client.update();

            if self.client.start() {
                self.act();
            }

            let pause = time::Duration::from_millis(1000);
            thread::sleep(pause);
        }
    }
}

fn main() {
    let addr_str = "127.0.0.1";
    let port_str = "8080";
    let game_key_str = "a";

    let uri: tungstenite::http::Uri = format!("ws://{}:{}", addr_str, port_str).parse().unwrap();
    let builder =
        tungstenite::ClientRequestBuilder::new(uri).with_header("ws_secret_key", game_key_str);
    let (mut ws, response) = tungstenite::connect(builder).unwrap();

    let client = Client::connect(ws);

    let mut bot = MarketMaker::new(client);
    bot.run();
}
