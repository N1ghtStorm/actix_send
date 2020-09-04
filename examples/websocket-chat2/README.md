# Websocket chat example

This is extension of the
[actix chat example](https://github.com/actix/actix/tree/master/examples/chat)

Added features:

* Browser WebSocket client

## Server

Server can access several types of message:

* `/list` - list all available rooms
* `/join name` - join room, if room does not exist, create new one
* `/name name` - set session name
* `some message` - just string, send a message to all peers in same room
* client has to send heartbeat `Ping` messages, if server does not receive a heartbeat message for 10 seconds connection 
will get dropped the next time a message come from client or the client closed the connection.

To start server use command: `cargo run`

## WebSocket Browser Client

Open url: [http://localhost:8080/](http://localhost:8080/)
