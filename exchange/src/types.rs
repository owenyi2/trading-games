use std::ops::Neg;

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
