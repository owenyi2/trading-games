use std::sync::{atomic, mpsc};

use exchange::book::Order;
use exchange::exchange::Broadcaster;
use exchange::types::*;

use protocol::ExchangeEvent;

pub struct SingleBroadcaster {
    tx: mpsc::Sender<ExchangeEvent>,

    counter: atomic::AtomicU64, // private_tx: mpsc::Sender<(AccountId, ExchangePrivateMessage)> // TODO
}

impl SingleBroadcaster {
    pub fn new(tx: mpsc::Sender<ExchangeEvent>) -> Self {
        let counter: atomic::AtomicU64 = atomic::AtomicU64::new(0);
        Self { tx, counter }
    }
}

impl Broadcaster for SingleBroadcaster {
    fn broadcast_insert(&mut self, order: &Order, message: &Message) {
        let id = self.counter.fetch_add(1, atomic::Ordering::Relaxed);

        let _ = self.tx.send(ExchangeEvent::Insert {
            price: order.price(),
            qty: order.qty(),
            side: order.side() as i8,
            id,
        });
    }
    fn broadcast_cancel(&mut self, order: &Order, message: &Message) {
        let id = self.counter.fetch_add(1, atomic::Ordering::Relaxed);

        let _ = self.tx.send(ExchangeEvent::Cancel {
            order_id: order.order_id(),
            id,
        });
    }
    fn broadcast_trade(
        &mut self,
        ask_order: &Order,
        bid_order: &Order,
        trade_price: Price,
        trade_volume: Quantity,
    ) {
        let id = self.counter.fetch_add(1, atomic::Ordering::Relaxed);

        let _ = self.tx.send(ExchangeEvent::Trade {
            ask_id: ask_order.order_id(),
            bid_id: bid_order.order_id(),
            trade_price,
            trade_volume,
            id,
        });
    }
}
