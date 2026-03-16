use serde::{Deserialize, Serialize};

use exchange::types::*;

pub type ExchangeId = u64;

type ExchangeEventId = u64;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ClientAction {
    CancelOrder {
        order_id: u64,
    },
    InsertOrder {
        side: bool,
        price: u64,
        qty: u64,
        client_order_id: u64,
    },
    End,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientMessage {
    pub exchange_id: u64,
    pub action: ClientAction,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMessage {
    Event(u64, ExchangeEvent),
    Private(u64, ExchangePrivateMessage),
    System(SystemMessage),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SystemMessage {
    Start,
    ExchangeAdded { exchange_id: u64 },
    AccountId { account_id: u64 },
    End,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ExchangeEvent {
    Cancel {
        order_id: OrderId,
        id: ExchangeEventId,
    },
    Insert {
        price: Price,
        qty: Quantity,
        side: i8,
        order_id: OrderId,
        id: ExchangeEventId,
    },
    Trade {
        ask_id: OrderId,
        bid_id: OrderId,
        trade_price: Price,
        trade_volume: Quantity,
        id: ExchangeEventId,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ExchangePrivateMessage {
    InsertConfirm {
        client_order_id: u64,
        order_id: OrderId,
        id: ExchangeEventId,
    },
    CancelConfirm {
        order_id: OrderId,
        id: ExchangeEventId,
    },
    TradeConfirm {
        order_id: OrderId,
        trade_price: Price,
        trade_volume: Quantity,
        side: i8,
        id: ExchangeEventId,
    },
}
