use std::cmp;
use std::sync::mpsc;

use crate::book::OrderBook;
use crate::types::{AccountId, Message, Order, OrderId, Price, Quantity, Side};

pub trait Broadcaster {
    fn broadcast_insert(&mut self, order: &Order, message: &Message);
    fn broadcast_cancel(&mut self, order: &Order, message: &Message);
    fn broadcast_trade(
        &mut self,
        ask_order: &Order,
        bid_order: &Order,
        trade_price: Price,
        trade_volume: Quantity,
    );
}

pub struct NoBroadcaster;

impl Broadcaster for NoBroadcaster {
    fn broadcast_insert(&mut self, order: &Order, message: &Message) {}
    fn broadcast_cancel(&mut self, order: &Order, message: &Message) {}
    fn broadcast_trade(
        &mut self,
        ask_order: &Order,
        bid_order: &Order,
        trade_price: Price,
        trade_volume: Quantity,
    ) {
    }
}

pub struct Exchange<Broadcaster> {
    order_book: OrderBook,
    pub broadcaster: Broadcaster,
    receiver: mpsc::Receiver<Message>,
}

impl<B: Broadcaster> Exchange<B> {
    pub fn new(broadcaster: B, receiver: mpsc::Receiver<Message>) -> Self {
        Self {
            order_book: OrderBook::default(),
            broadcaster,
            receiver,
        }
    }
    pub fn order_book(&self) -> &OrderBook {
        &self.order_book
    }
    pub fn summarise_book(&self) -> (Vec<(u64, u64)>, Vec<(u64, u64)>) {
        self.order_book.summarise()
    }
    pub fn run(&mut self) {
        loop {
            if let Ok(message) = self.receiver.try_recv() {
                match message {
                    Message::End => break,
                    Message::CancelOrder {
                        account_id,
                        order_id,
                    } => {
                        if let Some(order) = self.order_book.get_order(order_id)
                            && order.account_id() == account_id
                        {
                            self.broadcaster.broadcast_cancel(order, &message);
                            self.order_book.remove_order(
                                order_id,
                                order.price,
                                order.side,
                                order.qty,
                            );
                        }
                    }
                    Message::InsertOrder {
                        account_id,
                        price,
                        qty,
                        side,
                        ..
                    } => {
                        let order = Order::new_order(account_id, price, qty, side);
                        self.broadcaster.broadcast_insert(&order, &message);
                        self.order_book.insert_order(order);

                        while self.order_book.is_in_cross() {
                            let best_bid_id: OrderId = self
                                .order_book
                                .best_order(Side::Bid)
                                .expect("Orderbook is in cross"); // so these orders must exist
                            let best_ask_id: OrderId = self
                                .order_book
                                .best_order(Side::Ask)
                                .expect("Orderbook is in cross");

                            let best_bid = self
                                .order_book
                                .get_order(best_bid_id)
                                .expect("Orderbook is in cross");
                            let best_ask = self
                                .order_book
                                .get_order(best_ask_id)
                                .expect("Orderbook is in cross");
                            let trade_price = match best_bid_id.cmp(&best_ask_id) {
                                // price is limit of
                                // passive order
                                cmp::Ordering::Less => best_bid.price,
                                cmp::Ordering::Greater => best_ask.price,
                                cmp::Ordering::Equal => unreachable!(),
                            };
                            let trade_volume = cmp::min(best_bid.qty, best_ask.qty);
                            self.broadcaster.broadcast_trade(
                                best_ask,
                                best_bid,
                                trade_price,
                                trade_volume,
                            );
                            self.order_book
                                .match_order(best_bid_id, best_ask_id, trade_volume);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn order_book() {}

    #[test]
    fn exchange() {
        let (sender, receiver) = mpsc::channel();

        let broadcaster = NoBroadcaster::default();
        let mut exchange: Exchange<NoBroadcaster> = Exchange::new(broadcaster, receiver);
        let handle = thread::spawn(move || {
            exchange.run();
            exchange
        });

        sender.send(Message::InsertOrder {
            account_id: 0,
            side: Side::Bid,
            price: 100,
            qty: 10,
        });
        sender.send(Message::InsertOrder {
            account_id: 0,
            side: Side::Bid,
            price: 101,
            qty: 10,
        });
        sender.send(Message::InsertOrder {
            account_id: 0,
            side: Side::Ask,
            price: 102,
            qty: 10,
        });
        sender.send(Message::InsertOrder {
            account_id: 0,
            side: Side::Ask,
            price: 100,
            qty: 11,
        });
        sender.send(Message::End);

        let exchange = handle.join().unwrap();
        dbg!(&exchange.summarise_book());

        let expected = (vec![(100, 9)], vec![(102, 10)]);
        assert_eq!(exchange.summarise_book(), expected);
    }
}
