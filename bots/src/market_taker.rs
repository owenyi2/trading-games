use std::net::TcpStream;
use std::sync::{atomic, mpsc};
use std::thread::{self, JoinHandle};
use std::time;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use client_lib::Client;
use exchange::types::*;

use rand::Rng;
use rand::RngExt;
use rand_distr::{Distribution, Exp};
use tungstenite;
use tungstenite::stream::MaybeTlsStream;

struct MarketMaker {
    client: Client,
    last_change: Instant,
    lambda: f32,
}

impl MarketMaker {
    fn new(client: Client, lambda: f32) -> Self {
        Self {
            client,
            last_change: Instant::now(),
            lambda,
        }
    }
    fn act(&mut self) {
        let EXCHANGE_ID = 0;

        let mut rng = rand::rng();
        let coin_flip: bool = rng.random_bool(0.5);
        let qty = 10;

        let book = self.client.books().get(&EXCHANGE_ID).unwrap();

        let side = if coin_flip {
            if let Some(best) = book.order_book.best_order(Side::Ask) {
                let order = book.order_book.get_order(best).unwrap();
                self.client
                    .limit_order(EXCHANGE_ID, Side::Bid, order.price(), qty);
            }
        } else {
            if let Some(best) = book.order_book.best_order(Side::Bid) {
                let order = book.order_book.get_order(best).unwrap();
                self.client
                    .limit_order(EXCHANGE_ID, Side::Ask, order.price(), qty);
            }
        };
    }
    fn run(&mut self) {
        let exp = Exp::new(self.lambda).unwrap();
        let mut rng = rand::rng();

        let mut next_time = Instant::now() + Duration::from_secs_f32(exp.sample(&mut rng));

        loop {
            self.client.update();

            if self.client.start() {
                if Instant::now() >= next_time {
                    self.act();
                    next_time = Instant::now() + Duration::from_secs_f32(exp.sample(&mut rng));
                }
            }

            let pause = Duration::from_millis(10);
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

    let mut bot = MarketMaker::new(client, 1.0);
    bot.run();
}
