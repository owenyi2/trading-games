use std::collections::{HashMap, VecDeque};
use std::sync::atomic;

use crate::types::{AccountId, OrderId, Price, Quantity, Side};

#[derive(Debug)]
struct PriceLevel {
    orders: VecDeque<OrderId>,
    total_volume: Quantity,
    price: Price,
}

impl PriceLevel {
    fn new(price: Price) -> PriceLevel {
        PriceLevel {
            orders: VecDeque::new(),
            total_volume: 0,
            price,
        }
    }
    fn add_order(&mut self, order_id: OrderId, qty: Quantity) {
        self.total_volume += qty;
        self.orders.push_back(order_id);
    }
    fn reduce_volume(&mut self, volume: Quantity) {
        self.total_volume -= volume;
    }
    fn remove_order(&mut self, order_id: OrderId, qty: Quantity) -> Option<OrderId> {
        if let Some(index) = self.orders.iter().position(|x| *x == order_id) {
            self.total_volume -= qty; // qty represented by order_id to be removed
            return self.orders.remove(index);
        } else {
            return None;
        }
    }
}

#[derive(Default, Debug)]
pub struct OrderBook {
    bids: Vec<PriceLevel>,
    asks: Vec<PriceLevel>,
    order_info: HashMap<OrderId, Order>,
}

impl OrderBook {
    fn linear_search(list: &Vec<PriceLevel>, price: Price, side: Side) -> Option<usize> {
        match side {
            Side::Bid => {
                for (pos, level) in list.iter().enumerate().rev() {
                    if price >= level.price {
                        return Some(pos);
                    }
                }
            }
            Side::Ask => {
                for (pos, level) in list.iter().enumerate().rev() {
                    if price <= level.price {
                        return Some(pos);
                    }
                }
            }
        }
        return None;
    }

    pub fn best_order(&self, side: Side) -> Option<OrderId> {
        match side {
            Side::Bid => {
                let best_bid_level = self.bids.last()?;
                Some(*best_bid_level.orders.front().expect("volume > 0"))
            }
            Side::Ask => {
                let best_ask_level = self.asks.last()?;
                Some(*best_ask_level.orders.front().expect("volume > 0"))
            }
        }
    }

    pub fn remove_order(&mut self, order_id: OrderId, price: Price, side: Side, qty: Quantity) {
        match side {
            Side::Bid => {
                if let Some(pos) = Self::linear_search(&self.bids, price, side) {
                    self.bids[pos].remove_order(order_id, qty);
                    if self.bids[pos].total_volume == 0 {
                        self.bids.remove(pos);
                    }
                }
            }
            Side::Ask => {
                if let Some(pos) = Self::linear_search(&self.asks, price, side) {
                    self.asks[pos].remove_order(order_id, qty);
                    if self.asks[pos].total_volume == 0 {
                        self.asks.remove(pos);
                    }
                }
            }
        }
        self.order_info.remove(&order_id);
    }

    fn reduce_volume(&mut self, order_id: OrderId, price: Price, side: Side, volume: Quantity) {
        match side {
            Side::Bid => {
                if let Some(pos) = Self::linear_search(&self.bids, price, side) {
                    self.bids[pos].reduce_volume(volume);
                    self.order_info
                        .get_mut(&order_id)
                        .expect("`order_id` is present in self.bids")
                        .qty -= volume;
                }
            }
            Side::Ask => {
                if let Some(pos) = Self::linear_search(&self.asks, price, side) {
                    self.asks[pos].reduce_volume(volume);
                    self.order_info
                        .get_mut(&order_id)
                        .expect("`order_id` is present in self.asks")
                        .qty -= volume;
                }
            }
        }
    }

    pub fn match_order(&mut self, bid_id: OrderId, ask_id: OrderId, trade_volume: Quantity) {
        let orders: Option<(&Order, &Order)> = self
            .order_info
            .get(&bid_id)
            .zip(self.order_info.get(&ask_id));

        if let Some((bid, ask)) = orders {
            let (bid_price, ask_price) = (bid.price, ask.price);
            let (bid_qty, ask_qty) = (bid.qty, ask.qty);
            if bid_qty == trade_volume {
                self.remove_order(bid_id, bid_price, Side::Bid, bid_qty);
            } else {
                self.reduce_volume(bid_id, bid_price, Side::Bid, trade_volume);
            }

            if ask_qty == trade_volume {
                self.remove_order(ask_id, ask_price, Side::Ask, ask_qty);
            } else {
                self.reduce_volume(ask_id, ask_price, Side::Ask, trade_volume);
            }
        }
    }

    pub fn is_in_cross(&self) -> bool {
        self.bids
            .last()
            .and_then(|bid| self.asks.last().map(|ask| bid.price >= ask.price))
            .unwrap_or(false)
    }
    pub fn get_order(&self, order_id: OrderId) -> Option<&Order> {
        self.order_info.get(&order_id)
    }

    pub fn insert_order(&mut self, order: Order) {
        let price = order.price;

        match order.side {
            Side::Bid => {
                let pos = Self::linear_search(&self.bids, price, Side::Bid);

                if let Some(mut pos) = pos {
                    if self.bids[pos].price != price {
                        pos = pos + 1;
                        self.bids.insert(pos, PriceLevel::new(price));
                    }
                    self.bids[pos].add_order(order.order_id, order.qty);
                } else {
                    self.bids.insert(0, PriceLevel::new(price));
                    self.bids[0].add_order(order.order_id, order.qty);
                }
            }
            Side::Ask => {
                let pos = Self::linear_search(&self.asks, price, Side::Ask);

                if let Some(mut pos) = pos {
                    if self.asks[pos].price != price {
                        pos = pos + 1;
                        self.asks.insert(pos, PriceLevel::new(price));
                    }
                    self.asks[pos].add_order(order.order_id, order.qty);
                } else {
                    self.asks.insert(0, PriceLevel::new(price));
                    self.asks[0].add_order(order.order_id, order.qty);
                }
            }
        }
        self.order_info.insert(order.order_id, order);
    }
    pub fn summarise(&self) -> (Vec<(u64, u64)>, Vec<(u64, u64)>) {
        // Bids (price, volume), Asks (price, volume)
        (
            self.bids
                .iter()
                .map(|level| (level.price, level.total_volume))
                .collect(),
            self.asks
                .iter()
                .map(|level| (level.price, level.total_volume))
                .collect(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct Order {
    account_id: AccountId,
    pub(crate) side: Side,
    pub(crate) price: Price,
    pub(crate) qty: Quantity,
    order_id: OrderId,
}

impl Order {
    #[inline]
    pub fn side(&self) -> Side {
        self.side
    }
    #[inline]
    pub fn price(&self) -> Price {
        self.price
    }
    #[inline]
    pub fn qty(&self) -> Quantity {
        self.qty
    }
    #[inline]
    pub fn order_id(&self) -> Quantity {
        self.order_id
    }

    pub fn new_order(account_id: AccountId, price: Price, qty: Quantity, side: Side) -> Self {
        static COUNTER: atomic::AtomicU64 = atomic::AtomicU64::new(0);
        let order_id = COUNTER.fetch_add(1, atomic::Ordering::Relaxed);
        Order {
            account_id,
            price,
            qty,
            order_id: order_id,
            side,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn order_book() {}
}
