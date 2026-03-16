use std::collections::HashMap;
use std::sync::{atomic, mpsc};

use bus;

use exchange::exchange::Broadcaster;
use exchange::types::*;

use protocol::{ExchangeEvent, ExchangeId, ExchangePrivateMessage};

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
            order_id: order.order_id(),
        });

        match message {
            Message::InsertOrder {
                client_order_id, ..
            } => {
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
                            id,
                        },
                    ));
            }
            _ => unreachable!(),
        }
    }
    fn broadcast_cancel(&mut self, order: &Order, message: &Message) {
        let id = self.counter.fetch_add(1, atomic::Ordering::Relaxed);

        let _ = self.tx.broadcast(ExchangeEvent::Cancel {
            order_id: order.order_id(),
            id,
        });

        match message {
            Message::CancelOrder { order_id, .. } => {
                let _ = self
                    .private_txs
                    .get(&order.account_id()) // TODO: actually we need to map
                    // AccountId->ConnectionId->Sender
                    .expect("Orders must come from registered accounts")
                    .send((
                        self.exchange_id,
                        ExchangePrivateMessage::CancelConfirm {
                            order_id: order.order_id(),
                            id,
                        },
                    ));
            }
            _ => unreachable!(),
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

        for (side, order) in [(Side::Ask, ask_order), (Side::Bid, bid_order)] {
            let _ = self
                .private_txs
                .get(&order.account_id())
                .expect("Orders must come from registered accounts")
                .send((
                    self.exchange_id,
                    ExchangePrivateMessage::TradeConfirm {
                        order_id: order.order_id(),
                        trade_price,
                        trade_volume,
                        side: side as i8,
                        id,
                    },
                ));
        }
    }
}
