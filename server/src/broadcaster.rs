use std::collections::HashMap;
use std::sync::{atomic, mpsc};

use bus;

use exchange::book::Order;
use exchange::exchange::Broadcaster;
use exchange::types::*;

use protocol::{ConnectionId, ExchangeEvent, ExchangeId, ExchangePrivateMessage};

pub struct BusBroadcaster {
    pub tx: bus::Bus<ExchangeEvent>,
    pub private_txs: HashMap<u64, mpsc::Sender<(ExchangeId, ExchangePrivateMessage)>>,
    exchange_id: ExchangeId,
    counter: atomic::AtomicU64,
}

impl BusBroadcaster {
    pub fn new(exchange_id: ExchangeId, tx: bus::Bus<ExchangeEvent>) -> Self {
        let counter: atomic::AtomicU64 = atomic::AtomicU64::new(0);
        Self {
            exchange_id,
            tx,
            private_txs: HashMap::new(),
            counter,
        }
    }
}

impl Broadcaster for BusBroadcaster {
    fn broadcast_insert(&mut self, order: &Order, message: &Message) {
        let id = self.counter.fetch_add(1, atomic::Ordering::Relaxed);

        let _ = self.tx.broadcast(ExchangeEvent::Insert {
            price: order.price(),
            qty: order.qty(),
            side: order.side() as i8,
            id,
        });

        if let Message::InsertOrder {
            client_order_id, ..
        } = message
        {
            let _ = self
                .private_txs
                .get(&order.account_id()) // TODO: actually we need to map
                // AccountId->ConnectionId->Sender
                .expect("Orders must come from registered accounts")
                .send((
                    self.exchange_id,
                    ExchangePrivateMessage::InsertConfirm {
                        client_order_id: *client_order_id,
                        order_id: order.order_id(),
                    },
                ));
        }
    }
    fn broadcast_cancel(&mut self, order: &Order, message: &Message) {
        let id = self.counter.fetch_add(1, atomic::Ordering::Relaxed);

        let _ = self.tx.broadcast(ExchangeEvent::Cancel {
            order_id: order.order_id(),
            id,
        });
        if let Message::CancelOrder { order_id, .. } = message {
            let _ = self
                .private_txs
                .get(&order.account_id()) // TODO: actually we need to map
                // AccountId->ConnectionId->Sender
                .expect("Orders must come from registered accounts")
                .send((
                    self.exchange_id,
                    ExchangePrivateMessage::CancelConfirm {
                        order_id: order.order_id(),
                    },
                ));
        }
    }
    fn broadcast_trade(
        &mut self,
        ask_order: &Order,
        bid_order: &Order,
        trade_price: Price,
        trade_volume: Quantity,
    ) {
        let id = self.counter.fetch_add(1, atomic::Ordering::Relaxed);

        let _ = self.tx.broadcast(ExchangeEvent::Trade {
            ask_id: ask_order.order_id(),
            bid_id: bid_order.order_id(),
            trade_price,
            trade_volume,
            id,
        });
    }
}
