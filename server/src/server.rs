use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, atomic, mpsc};
use std::thread;
use std::time::Duration;

use bus;
use rmp_serde;
use tungstenite;

use exchange::exchange::Exchange;
use exchange::types::*;

use crate::broadcaster::SingleBroadcaster;

use protocol::{ClientAction, ClientMessage, ExchangeEvent, ServerMessage, SystemMessage};

type ExchangeId = u64;
type ConnectionId = u64;

#[derive(Clone, Copy)]
enum Command {
    StopAcceptingConnections,
    AddExchange,
    Synchronise,
    Shutdown,
}

fn parse_command(input: &str) -> Result<Command, String> {
    let parts: Vec<&str> = input.trim().split_whitespace().collect();

    match parts.as_slice() {
        ["stop_accepting_connections"] => Ok(Command::StopAcceptingConnections),
        ["add_exchange"] => Ok(Command::AddExchange),
        ["sync"] => Ok(Command::Synchronise),
        ["shutdown"] => Ok(Command::Shutdown),
        _ => Err("Unknown command".into()),
    }
}

enum ShellMessage {
    NewExchange {
        exchange_id: ExchangeId,
        msg_sender: mpsc::Sender<Message>,
        event_receiver: bus::BusReader<ExchangeEvent>,
    },
    Stop,
}

struct ConnectionHandler {
    websocket: tungstenite::WebSocket<TcpStream>,
    msg_senders: HashMap<ExchangeId, mpsc::Sender<Message>>,
    event_receiver: HashMap<ExchangeId, bus::BusReader<ExchangeEvent>>,

    shell_receiver: mpsc::Receiver<ShellMessage>,
}

impl ConnectionHandler {
    fn new(
        websocket: tungstenite::WebSocket<TcpStream>,
        shell_receiver: mpsc::Receiver<ShellMessage>,
    ) -> Self {
        Self {
            websocket,
            msg_senders: HashMap::new(),
            event_receiver: HashMap::new(),
            shell_receiver,
        }
    }
    pub fn parse_message(input: &str) -> Result<(ExchangeId, Message), ()> {
        let parts: Vec<&str> = input.trim().split_whitespace().collect();
        match parts.as_slice() {
            [
                "exchange",
                exchange,
                "insert",
                account_id,
                side_str,
                price,
                qty,
                client_order_id,
            ] => {
                // Parse account_id
                let account_id: AccountId = account_id.parse().map_err(|_| ())?;

                // Parse side
                let side = match *side_str {
                    "Bid" | "bid" => Side::Bid,
                    "Ask" | "ask" => Side::Ask,
                    _ => return Err(()),
                };

                // Parse price and quantity
                let price: Price = price.parse().map_err(|_| ())?;
                let qty: Quantity = qty.parse().map_err(|_| ())?;
                let exchange: ExchangeId = exchange.parse().map_err(|_| ())?;
                let client_order_id: u64 = client_order_id.parse().map_err(|_| ())?;

                Ok((
                    exchange,
                    Message::InsertOrder {
                        account_id,
                        side,
                        price,
                        qty,
                        client_order_id,
                    },
                ))
            }

            // You can add more patterns for cancel or end messages here
            _ => Err(()),
        }
    }

    fn run(&mut self) {
        loop {
            // 1. Check shell_receiver
            if let Ok(msg) = self.shell_receiver.try_recv() {
                match msg {
                    ShellMessage::NewExchange {
                        exchange_id,
                        msg_sender,
                        event_receiver,
                    } => {
                        self.msg_senders.insert(exchange_id, msg_sender);
                        self.event_receiver.insert(exchange_id, event_receiver);

                        let server_message =
                            ServerMessage::System(SystemMessage::ExchangeAdded { exchange_id });
                        let encoded: Vec<u8> = rmp_serde::to_vec(&server_message).unwrap();
                        let _ = self
                            .websocket
                            .send(tungstenite::Message::Binary(encoded.into()));
                    }
                    ShellMessage::Stop => {
                        break;
                    }
                }
            }
            // 2. Check each exchange event
            for (exchange_id, reader) in &mut self.event_receiver {
                while let Ok(event) = reader.try_recv() {
                    let server_message = ServerMessage::Event(event);
                    let encoded: Vec<u8> = rmp_serde::to_vec(&server_message).unwrap();
                    let ws_msg = tungstenite::Message::Binary(encoded.into());
                    if let Err(e) = self.websocket.send(ws_msg) {
                        eprintln!("WebSocket send error: {:?}", e);
                    }
                }
            }
            // 3. Check the websocket for incoming messages
            if let Ok(msg) = self.websocket.read() {
                if msg.is_binary() {
                    if let Ok(parsed) = rmp_serde::from_slice::<ClientMessage>(&msg.into_data()) {
                        if let Some(msg_sender) = self.msg_senders.get_mut(&parsed.exchange_id) {
                            let account_id = parsed.account_id;
                            let order = match parsed.action {
                                ClientAction::CancelOrder { order_id } => Message::CancelOrder {
                                    order_id,
                                    account_id,
                                },
                                ClientAction::InsertOrder {
                                    side,
                                    price,
                                    qty,
                                    client_order_id,
                                } => Message::InsertOrder {
                                    account_id,
                                    price,
                                    qty,
                                    client_order_id,
                                    side: match side {
                                        true => Side::Bid,
                                        false => Side::Ask,
                                    },
                                },
                            };
                            msg_sender.send(order);
                        }
                    }
                }
            }
        }
    }
}

type Connections = HashMap<ConnectionId, mpsc::Sender<ShellMessage>>;

pub struct Server {
    stop_accepting_connections: Arc<atomic::AtomicBool>,

    connections: Arc<Mutex<Connections>>,

    exchanges: HashMap<ExchangeId, (mpsc::Sender<Message>, Arc<Mutex<bus::Bus<ExchangeEvent>>>)>,
}

impl Server {
    pub fn new() -> Self {
        Server {
            // message_senders: HashMap::new(),
            // event_buses: HashMap::new(),
            exchanges: HashMap::new(),
            stop_accepting_connections: Arc::new(atomic::AtomicBool::new(false)),
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn process_command(&mut self, cmd: Command) {
        match cmd {
            Command::StopAcceptingConnections => {
                self.stop_accepting_connections
                    .store(true, atomic::Ordering::Relaxed);
            }
            Command::AddExchange => {
                self.add_exchange();
            }
            Command::Synchronise => {
                self.sync_connections_to_exchanges();
            }
            Command::Shutdown => {
                todo!()
            }
        }
    }

    fn init(&mut self) {
        let connections = self.connections.clone();
        let stop_accepting = self.stop_accepting_connections.clone();
        let manage_connections = thread::spawn(move || {
            Self::accept_connections(connections, stop_accepting);
        });
    }

    pub fn run(&mut self) {
        self.init();
        self.shell();
    }

    fn shell(&mut self) {
        loop {
            print!("> ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            match io::stdin().read_line(&mut input) {
                Ok(_) => {
                    let input = input.trim();

                    // skip empty input
                    if input.is_empty() {
                        continue;
                    }
                    if let Ok(cmd) = parse_command(&input) {
                        self.process_command(cmd);
                        if matches!(cmd, Command::Shutdown) {
                            break;
                        }
                    }
                }
                Err(error) => {
                    eprintln!("Error reading input: {}", error);
                    break;
                }
            }
        }
    }

    fn add_exchange(&mut self) {
        static COUNTER: atomic::AtomicU64 = atomic::AtomicU64::new(0);
        let exchange_id = COUNTER.fetch_add(1, atomic::Ordering::Relaxed);

        let (msg_sender, msg_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();

        let broadcaster = SingleBroadcaster::new(event_sender);
        let mut exchange: Exchange<SingleBroadcaster> = Exchange::new(broadcaster, msg_receiver);

        let _ = thread::spawn(move || {
            exchange.run();
        });

        let bus = Arc::new(Mutex::new(bus::Bus::<ExchangeEvent>::new(256)));
        let bus_clone = bus.clone();

        self.exchanges
            .insert(exchange_id, (msg_sender.clone(), bus.clone()));

        let _ = thread::spawn(move || {
            while let Ok(event) = event_receiver.recv() {
                let mut bus_guard = bus_clone.lock().unwrap();
                bus_guard.broadcast(event);
            }
        });
    }

    fn sync_connections_to_exchanges(&mut self) {
        for (client_id, tx) in self.connections.lock().unwrap().iter() {
            for (exchange_id, (msg_sender, bus)) in self.exchanges.iter() {
                let _ = tx.send(ShellMessage::NewExchange {
                    exchange_id: *exchange_id,
                    msg_sender: msg_sender.clone(),
                    event_receiver: bus.lock().unwrap().add_rx(),
                });
            }
        }
    }

    fn accept_connections(
        connections: Arc<Mutex<Connections>>,
        stop_accepting: Arc<atomic::AtomicBool>,
    ) {
        let addr = "127.0.0.1:8080";
        let listener = TcpListener::bind(addr).expect("Failed to bind");

        while !stop_accepting.load(atomic::Ordering::Relaxed) {
            let (stream, _) = listener.accept().expect("TCP Connection Failed");
            let mut websocket = tungstenite::accept(stream).expect("websocket connection failed");

            static COUNTER: atomic::AtomicU64 = atomic::AtomicU64::new(0);
            let connection_id = COUNTER.fetch_add(1, atomic::Ordering::Relaxed);

            let (tx, rx) = mpsc::channel();
            connections.lock().unwrap().insert(connection_id, tx);

            websocket
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();

            let mut connection_handler = ConnectionHandler::new(websocket, rx);
            let _ = thread::spawn(move || {
                connection_handler.run();
            });
        }
    }
}
