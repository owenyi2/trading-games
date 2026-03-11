import asyncio
import websockets
import sys
import msgpack
import itertools
from dotenv import load_dotenv

import os

load_dotenv()
WS_SECRET_KEY = os.getenv("WS_SECRET_KEY")

client_id = 69
order_id_counter = itertools.count(1)  # auto-increment order IDs

"""
Insert an order by typing something like

`bid 100 10`

to mean "Bid" with price 100 for 10 lots

Specify an Exchange by typing something like

`0 bid 100 10` to mean "Bid" with price 100 for 10 lots on exchange 0
"""

async def receiver(ws):
    """Receive messages from websocket and print them."""
    try:
        async for message in ws:
            # Move to new line so it doesn't break user input
            sys.stdout.write("\r")
            print(f"\n<< {msgpack.unpackb(message, raw = False)}")
            print(">> ", end="", flush=True)
    except websockets.ConnectionClosed:
        print("\nConnection closed.")

async def sender(ws):
    """Read user input and send to websocket."""
    loop = asyncio.get_event_loop()
    while True:
        # Run blocking input() in executor so it doesn't block event loop
        message = await loop.run_in_executor(None, input, ">> ")

        if message.lower() in ("exit", "quit"):
            await ws.close()
            break
        parts = message.split(" ")
        if len(parts) == 3:
            try:
                order_id = next(order_id_counter)
                if parts[0].lower() == "bid":
                    side = True
                elif parts[0].lower() == "ask":
                    side = False
                else: 
                    raise ValueError
                message = {
                    "exchange_id": 0,
                    "account_id": client_id,
                    "action": {
                        "InsertOrder": {
                            "side": side,
                            "price": int(parts[1]),
                            "qty": int(parts[2]),
                            "client_order_id": order_id
                            }
                        }
                    }
                pack = msgpack.packb(message) 
                await ws.send(pack)
            except:
                continue
        if len(parts) == 4:
            try:
                order_id = next(order_id_counter)
                if parts[1].lower() == "bid":
                    side = True
                elif parts[1].lower() == "ask":
                    side = False
                else: 
                    raise ValueError
                message = {
                    "exchange_id": int(parts[0]),
                    "account_id": client_id,
                    "action": {
                        "InsertOrder": {
                            "side": side,
                            "price": int(parts[2]),
                            "qty": int(parts[3]),
                            "client_order_id": order_id
                            }
                        }
                    }
                pack = msgpack.packb(message) 
                await ws.send(pack)
            except:
                continue

async def main():
    uri = "ws://localhost:8080"  # Change to your websocket server
    async with websockets.connect(uri, additional_headers = { "ws_secret_key": WS_SECRET_KEY }) as ws:
        print(f"Connected to {uri}")
        await asyncio.gather(
            receiver(ws),
            sender(ws),
        )

if __name__ == "__main__":
    asyncio.run(main())
