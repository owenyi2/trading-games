mod broadcaster;
// mod server; // TODO: move all theses structs into mod server;

use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, atomic, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::fmt;

use bus;
use rmp_serde;
use tungstenite;

use exchange::exchange::Exchange;
use exchange::types::*;

use broadcaster::{BusBroadcaster,ExchangeId,ConnectionId};

use protocol::{
    ClientAction, ClientMessage, ExchangeEvent, ExchangePrivateMessage, ServerMessage,
    SystemMessage,
};


#[derive(Debug)]
struct ConnectionHandler {
    id: ConnectionId,
    ws_tx: mpsc::Sender<ServerMessage>,
    ws_rx: mpsc::Receiver<ClientMessage>,

    private_rx: Option<mpsc::Receiver<(ExchangeId, ExchangePrivateMessage)>>,
    bus_readers: HashMap<ExchangeId, bus::BusReader<ExchangeEvent>>,
    msg_senders: HashMap<ExchangeId, mpsc::Sender<Message>> 
}

impl ConnectionHandler {
    fn new(id: ConnectionId, ws_tx: mpsc::Sender<ServerMessage>, ws_rx: mpsc::Receiver<ClientMessage>) -> Self {
        ConnectionHandler { id, ws_tx, ws_rx, private_rx: None, bus_readers: HashMap::new(), msg_senders: HashMap::new() }
    }
    fn run(&mut self, stop_receiver: mpsc::Receiver<SystemMessage>) {
        loop {
            // 0. Check if server wants to stop us
            if let Ok(msg) = stop_receiver.try_recv() {
                match msg {
                    SystemMessage::End => break,
                    _ => unreachable!()
                }
            }
            // 1. Check each exchange event
            for (exchange_id, reader) in &mut self.bus_readers{
                while let Ok(event) = reader.try_recv() {
                    let server_message = ServerMessage::Event(*exchange_id, event);
                    let _ = self.ws_tx.send(server_message);
                }
            }
            // 2. Check for private exchange messages
            if let Some(rx) = &self.private_rx {
                // self.private_rx is wrapped in an option because the rx is added later in the
                // objects lifetime by Server.sync
                while let Ok((exchange_id, msg)) = rx.try_recv() {
                    let server_message = ServerMessage::Private(exchange_id, msg);
                    let _ = self.ws_tx.send(server_message);
                }
            }
            // 3. Check the websocket for incoming messages
            while let Ok(msg) = self.ws_rx.try_recv() {
                if let Some(msg_sender) = self.msg_senders.get_mut(&msg.exchange_id) {
                    let account_id = self.id; // ignore the msg.account_id TODO: remove that from
                                              // protocol::ClientMessage
                    let order = match msg.action {
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

type Connections = HashMap<ConnectionId, ConnectionHandler>;

struct ExchangeHandler {
    exchange: Exchange<BusBroadcaster>,
    msg_sender: mpsc::Sender<Message>,
}
impl fmt::Debug for ExchangeHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExchangeHandler")
            .finish()
    }
}

#[derive(Clone, Copy)]
enum Command {
    Stop,
    Play,
    Pause,
    AddExchange
}

fn parse_command(input: &str) -> Result<Command, String> {
    let parts: Vec<&str> = input.trim().split_whitespace().collect();

    match parts.as_slice() {
        ["play"] => Ok(Command::Play),
        ["pause"] => Ok(Command::Pause),
        ["add_exchange"] => Ok(Command::AddExchange),
        ["stop"] => Ok(Command::Stop),
        _ => Err("Unknown command".into()),
    }
}

struct RunningServer {
    connections: HashMap<ConnectionId, JoinHandle<ConnectionHandler>>,
    exchanges: HashMap<ExchangeId, JoinHandle<ExchangeHandler>>,

    connection_stoppers: HashMap<ConnectionId, mpsc::Sender<SystemMessage>>,
    exchange_stoppers: HashMap<ExchangeId, mpsc::Sender<Message>>

}

impl RunningServer {
    fn run(self) -> PausedServer {
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
                        match cmd {
                            Command::Pause => {
                                return self.create_paused_server();
                            }
                            _ => {
                            }
                        }
                    } else {
                        eprintln!("Unrecognised Command");
                    }
                }
                Err(error) => {
                    eprintln!("Error reading input: {}", error);
                    continue;
                }
            }
        }
    }

    fn create_paused_server(self) -> PausedServer { 
        let mut paused_connections = HashMap::new();

        for (_, stopper) in self.connection_stoppers {
            stopper.send(SystemMessage::End);
        }
        for (connection_id, connection) in self.connections {
            paused_connections.insert(
                connection_id, connection.join().unwrap()
            );
        }
        
        let mut paused_exchanges = HashMap::new();
        for (_, stopper) in self.exchange_stoppers {
            stopper.send(Message::End);
        }
        for (exchange_id, exchange) in self.exchanges {
            paused_exchanges.insert(
                exchange_id, exchange.join().unwrap()
            );
        }

        PausedServer::new(
            paused_connections, paused_exchanges
            ) 
    }
}

#[derive(Debug)]
struct PausedServer {
    stop: Arc<atomic::AtomicBool>,
    connections: Arc<Mutex<Connections>>,
    exchanges: HashMap<ExchangeId, ExchangeHandler>,
    accept_handle: Option<JoinHandle<()>>
}

impl PausedServer {
    fn new(connections: Connections, exchanges: HashMap<ExchangeId, ExchangeHandler>) -> Self {
        let stop = Arc::new(atomic::AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(connections));

        let stop_clone = stop.clone();
        let connections_clone = connections.clone();
    
        let accept_handle = thread::spawn(move || {
            Self::accept_connections(connections_clone, stop_clone);
        });

        PausedServer {
            stop,
            connections,
            exchanges,
            accept_handle: Some(accept_handle), 
        }
    }
    fn create_running_server(self) -> RunningServer {
        let Self { exchanges, connections, .. } = self;

        let mut running_exchanges = HashMap::new();
        let mut running_connections = HashMap::new();

        let mut exchange_stoppers = HashMap::new();
        let mut connection_stoppers = HashMap::new();

        for (exchange_id, mut exchange_handler) in exchanges {
            exchange_stoppers.insert(
                exchange_id, exchange_handler.msg_sender.clone()
                );
            running_exchanges.insert(
                exchange_id, thread::spawn(move || {
                    exchange_handler.exchange.run();
                    exchange_handler
            })
            );
        }
        let mut connections_inner = connections.lock().unwrap();
        for (connection_id, mut connection_handler) in connections_inner.drain() {
            let (stop_tx, stop_rx) = mpsc::channel();
            connection_stoppers.insert(
                connection_id, stop_tx
                );
            running_connections.insert(
                connection_id, thread::spawn(move || {
                    connection_handler.run(stop_rx);
                    connection_handler
                })
                );
        }
        RunningServer {
            exchanges: running_exchanges, connections: running_connections,
            exchange_stoppers, connection_stoppers
        }
    } 

    fn run(mut self) -> Option<RunningServer> {
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
                        match cmd {
                            Command::Stop => {
                                self.stop.store(true, atomic::Ordering::Relaxed);
                                let _ = self.accept_handle.take().unwrap().join();
                                break;
                            }
                            Command::AddExchange => {
                                self.add_exchange();
                            }
                            Command::Play => {
                                self.stop.store(true, atomic::Ordering::Relaxed);
                                let _ = self.accept_handle.take().unwrap().join();
                                self.sync();
                                return Some(self.create_running_server());
                            }
                            _ => {
                            }
                        }
                    } else {
                        eprintln!("Unrecognised Command");
                    }
                }
                Err(error) => {
                    eprintln!("Error reading input: {}", error);
                    break;
                }
            }
        }
        None
    }
    fn add_exchange(&mut self) {
        let exchange_id: u64 = self.exchanges.len().try_into().unwrap();

        let (msg_sender, msg_receiver) = mpsc::channel();
        let bus = bus::Bus::<ExchangeEvent>::new(256);

        let broadcaster = BusBroadcaster::new(exchange_id, bus);
        let mut exchange: Exchange<BusBroadcaster> = Exchange::new(broadcaster, msg_receiver);
        self.exchanges.insert(
            exchange_id,
            ExchangeHandler {
                exchange,
                msg_sender,
            },
        );
    } 

    fn accept_connections(
        connections: Arc<Mutex<Connections>>,
        stop_accepting: Arc<atomic::AtomicBool>
        ) {
            static COUNTER: atomic::AtomicU64 = atomic::AtomicU64::new(0);
    
            let addr = "127.0.0.1:8080";
            let listener = TcpListener::bind(addr).expect("Failed to bind");
    
            listener.set_nonblocking(true).expect("Cannot set non-blocking");
    
            for stream in listener.incoming() {
                match stream {
                    Ok(s) => {
                        s.set_nonblocking(false).expect("Cannot set non-blocking");
                        let mut websocket = tungstenite::accept(s).expect("Websocket Connection Failed");
                        let connection_id = COUNTER.fetch_add(1, atomic::Ordering::Relaxed);
                        let (server_tx, server_rx) = mpsc::channel();
                        let (client_tx, client_rx) = mpsc::channel();
    
                        let ws_thread = thread::spawn(move || {
                            websocket.get_mut().set_nonblocking(true).expect("Cannot set non-blocking");
                            loop {
                                if let Ok(msg) = server_rx.try_recv() {
                                    if matches!(msg, ServerMessage::System(SystemMessage::End)) {
                                        break;
                                    }
                                    let encoded: Vec<u8> = rmp_serde::to_vec(&msg).unwrap();
                                    let _ = websocket.send(tungstenite::Message::Binary(encoded.into()));
                                }
                                if let Ok(msg) = websocket.read() {
                                    if msg.is_binary() {
                                        if let Ok(parsed) = rmp_serde::from_slice::<ClientMessage>(&msg.into_data()) {
                                            let _ = client_tx.send(parsed);
    
                                        }
                                    }
                                }
    
                            }
                        });
                        connections.lock().unwrap().insert(connection_id, ConnectionHandler::new( 
                            connection_id,
                            server_tx,
                            client_rx,
                        ));
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
    
                    }
                    Err(e) => panic!("Encountered IO error: {e}"),
                
                }
                if stop_accepting.load(atomic::Ordering::Relaxed) {
                    break;
                }
            }
    }
    fn sync(&mut self) {
        let mut connections_lock = self.connections.lock().unwrap();
        for (connection_id, connection) in connections_lock.iter_mut() {
            let (private_tx, private_rx) = mpsc::channel();
            for (exchange_id, exchange) in self.exchanges.iter_mut() {
                // Each Connection needs to subscribe a bus reader to each Exchange
                connection.bus_readers.insert(
                    *exchange_id, exchange.exchange.broadcaster.tx.add_rx()
                    );
                // Each Connection needs to clone a msg_sender to each Exchange
                connection.msg_senders.insert(
                    *exchange_id, exchange.msg_sender.clone()
                    );
                // Each Exchange.broadcaster needs to populate its private_txs

                exchange.exchange.broadcaster.private_txs.insert(
                    *connection_id, private_tx.clone());
            }
            // Each connection needs to have private_rx
            connection.private_rx = Some(private_rx);
        }
    }
}

fn main() {
    let mut paused_server = PausedServer::new(HashMap::new(), HashMap::new());    
    
    loop {
        let Some(mut running_server) = paused_server.run() else {
            // state machine cycles between Paused Server or Running Server
            break;
        };
        paused_server = running_server.run();
    }
}
