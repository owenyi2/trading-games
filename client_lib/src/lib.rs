use exchange::book::OrderBook;
use exchange::types::*;
use protocol::{
    ClientAction, ClientMessage, ConnectionId, ExchangeEvent, ExchangeId, ExchangePrivateMessage,
    ServerMessage, SystemMessage,
};

use std::collections::{HashMap, HashSet};
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use tungstenite;
use tungstenite::stream::MaybeTlsStream;

pub struct Client {
    ws_tx: mpsc::Sender<ClientMessage>,
    ws_rx: mpsc::Receiver<ServerMessage>,
    books: HashMap<ExchangeId, ClientOrderBook>,
    account_id: Option<u64>,
    client_order_id_counter: u64,
    start: bool,

    ws_thread: JoinHandle<tungstenite::WebSocket<MaybeTlsStream<TcpStream>>>,
}

impl Client {
    pub fn connect(mut ws: tungstenite::WebSocket<MaybeTlsStream<TcpStream>>) -> Self {
        let (server_tx, ws_rx) = mpsc::channel::<ServerMessage>();
        let (ws_tx, client_rx) = mpsc::channel::<ClientMessage>();

        let ws_thread = thread::spawn(move || {
            match ws.get_mut() {
                MaybeTlsStream::Plain(tcp) => {
                    tcp.set_nonblocking(true).expect("Cannot set non-blocking");
                }
                MaybeTlsStream::NativeTls(tls) => {
                    tls.get_mut()
                        .set_nonblocking(true)
                        .expect("Cannot set non-blocking");
                }
                _ => {
                    todo!()
                }
            }
            loop {
                if let Ok(msg) = client_rx.try_recv() {
                    let encoded: Vec<u8> = rmp_serde::to_vec(&msg).unwrap();
                    let _ = ws.send(tungstenite::Message::Binary(encoded.into()));
                }
                if let Ok(msg) = ws.read() {
                    if msg.is_binary() {
                        if let Ok(parsed) = rmp_serde::from_slice::<ServerMessage>(&msg.into_data())
                        {
                            let _ = server_tx.send(parsed);
                        }
                    }
                }
            }
            ws
        });
        Self {
            ws_tx,
            ws_rx,
            books: HashMap::new(),
            client_order_id_counter: 0,
            account_id: None,
            start: false,
            ws_thread,
        }
    }
    pub fn start(&self) -> bool {
        self.start
    }
    pub fn cancel_all(&self, exchange_id: ExchangeId) -> Option<()> {
        let book = self.books.get(&exchange_id)?;
        for order_id in &book.our_orders {
            let action = ClientAction::CancelOrder {
                order_id: *order_id,
            };
            let message = ClientMessage {
                exchange_id,
                action,
            };
            self.ws_tx.send(message);
        }
        Some(())
    }
    pub fn cancel_level(&self, exchange_id: ExchangeId, price: Price) -> Option<()> {
        todo!()
    }
    pub fn cancel_order(&self, exchange_id: ExchangeId, order_id: OrderId) -> Option<&Order> {
        let order = self
            .books
            .get(&exchange_id)?
            .order_book
            .get_order(order_id)?;
        let action = ClientAction::CancelOrder { order_id: order_id };
        let message = ClientMessage {
            exchange_id,
            action,
        };
        self.ws_tx.send(message);
        Some(order)
    }
    pub fn limit_order(
        &mut self,
        exchange_id: ExchangeId,
        side: Side,
        price: Price,
        qty: Quantity,
    ) {
        let client_order_id = self.client_order_id_counter;
        self.client_order_id_counter += 1;
        let side = match side {
            Side::Bid => true,
            Side::Ask => false,
        };

        let action = ClientAction::InsertOrder {
            side,
            price,
            qty,
            client_order_id,
        };
        let message = ClientMessage {
            exchange_id,
            action,
        };

        // TODO track our orders
        self.ws_tx.send(message);
    }

    pub fn update(&mut self) {
        while let Ok(server_msg) = self.ws_rx.try_recv() {
            match server_msg {
                ServerMessage::Event(id, event) => {
                    let book = self.books.get_mut(&id).unwrap();
                    book.handle_exchange_event(event);
                }
                ServerMessage::Private(id, private_msg) => {
                    let book = self.books.get_mut(&id).unwrap();
                    book.handle_exchange_private_message(private_msg);
                }
                ServerMessage::System(msg) => match msg {
                    SystemMessage::Start => {
                        self.start = true;
                    }
                    SystemMessage::ExchangeAdded { exchange_id } => {
                        self.books.insert(exchange_id, ClientOrderBook::new());
                    }
                    SystemMessage::AccountId { account_id } => {
                        self.account_id = Some(account_id);
                    }
                    SystemMessage::End => {
                        self.start = false;
                        todo!()
                    }
                },
            }
        }
    }
    pub fn books(&self) -> &HashMap<ExchangeId, ClientOrderBook> {
        &self.books
    }
    pub fn account_id(&self) -> Option<AccountId> {
        self.account_id
    }
}

pub struct ClientOrderBook {
    pub order_book: OrderBook,
    pub our_orders: HashSet<OrderId>,
    pub cash: i64,
    pub position: i64,
}

impl ClientOrderBook {
    fn new() -> Self {
        ClientOrderBook {
            order_book: OrderBook::default(),
            our_orders: HashSet::new(),
            cash: 0,
            position: 0,
        }
    }
    fn handle_exchange_event(&mut self, event: ExchangeEvent) {
        match event {
            ExchangeEvent::Cancel { order_id, .. } => match self.order_book.get_order(order_id) {
                Some(order) => {
                    self.order_book.remove_order(
                        order_id,
                        order.price(),
                        order.side(),
                        order.qty(),
                    );
                }
                None => panic!(
                    "client book out of sync with server. Missing order: {}",
                    order_id
                ),
            },
            ExchangeEvent::Insert {
                price, qty, side, ..
            } => {
                let side = match side {
                    1 => Side::Bid,
                    -1 => Side::Ask,
                    _ => panic!("side must be either 1 or -1"),
                };
                let order = Order::new_order(0, price, qty, side);
                self.order_book.insert_order(order);
                // TODO: it is possible that the orderbook is in cross before we receive the trade
                // message. Especially if there is lag spike. This could be problematic.
            }
            ExchangeEvent::Trade {
                ask_id,
                bid_id,
                trade_volume,
                ..
            } => {
                self.order_book.match_order(bid_id, ask_id, trade_volume);
            }
        }
    }

    fn handle_exchange_private_message(&mut self, message: ExchangePrivateMessage) {
        match message {
            ExchangePrivateMessage::InsertConfirm { order_id, .. } => {
                self.our_orders.insert(order_id);
            }
            ExchangePrivateMessage::CancelConfirm { order_id, .. } => {
                self.our_orders.remove(&order_id);
            }
            ExchangePrivateMessage::TradeConfirm {
                order_id,
                trade_price,
                trade_volume,
                side,
                ..
            } => {
                self.our_orders.remove(&order_id);
                let side = match side {
                    1 => Side::Bid,
                    -1 => Side::Ask,
                    _ => panic!("side must be either 1 or -1"),
                };

                match side {
                    Side::Bid => {
                        self.cash -= (trade_price * trade_volume) as i64;
                        self.position += trade_volume as i64;
                    }
                    Side::Ask => {
                        self.cash += (trade_price * trade_volume) as i64;
                        self.position -= trade_volume as i64;
                    }
                }
            }
        }
    }
}
