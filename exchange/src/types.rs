use std::ops::Neg;
use std::sync::atomic;

pub type AccountId = u64;
pub type Price = u64;
pub type Quantity = u64;
pub type OrderId = u64;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Side {
    Bid = 1,
    Ask = -1,
}

impl Neg for Side {
    type Output = Side;

    fn neg(self) -> Side {
        match self {
            Side::Bid => Side::Ask,
            Side::Ask => Side::Bid,
        }
    }
}

#[derive(Debug)]
pub enum Message {
    CancelOrder {
        order_id: OrderId,
        account_id: AccountId,
    },
    InsertOrder {
        account_id: AccountId,
        side: Side,
        price: Price,
        qty: Quantity,
        client_order_id: u64,
    },
    End,
}

#[derive(Debug, Clone)]
pub struct Order {
    account_id: AccountId,
    pub(crate) side: Side,
    pub(crate) price: Price,
    pub(crate) qty: Quantity,
    pub(crate) order_id: OrderId,
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
    #[inline]
    pub fn account_id(&self) -> AccountId {
        self.account_id
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
    pub fn new_order_with_order_id(
        account_id: AccountId,
        price: Price,
        qty: Quantity,
        side: Side,
        order_id: OrderId,
    ) -> Self {
        Order {
            account_id,
            price,
            qty,
            order_id: order_id,
            side,
        }
    }
}
