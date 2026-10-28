//! loop-bus: the hub. Every line a service writes is repeated to every other connected service.
//!
//!   loop-bus              run the hub
//!   loop-bus tap          print every event on the bus
//!   loop-bus send JSON    send one event, e.g. '{"type":"record","on":true}'
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

#[tokio::main(flavor = "current_thread")]
async fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	match args.first().map(String::as_str) {
		None => hub().await,
		Some("tap") => tap().await,
		Some("send") if args.len() == 2 => send(&args[1]).await,
		_ => {
			eprintln!("usage: loop-bus [tap | send JSON]");
			std::process::exit(2);
		}
	}
}

async fn tap() {
	let stream = UnixStream::connect(loop_bus::socket_path())
		.await
		.expect("connect to the loop bus (is the hub running?)");
	let mut lines = BufReader::new(stream).lines();
	while let Ok(Some(line)) = lines.next_line().await {
		println!("{line}");
	}
}

async fn send(json: &str) {
	let event: loop_bus::Event = serde_json::from_str(json).expect("an event, e.g. {\"type\":\"record\",\"on\":true}");
	let line = serde_json::to_string(&loop_bus::Envelope {
		from: "loop-bus send".into(),
		event,
	})
	.unwrap();
	let mut stream = UnixStream::connect(loop_bus::socket_path())
		.await
		.expect("connect to the loop bus (is the hub running?)");
	stream.write_all(format!("{line}\n").as_bytes()).await.unwrap();
	stream.flush().await.unwrap();
	// Give the hub a moment to read the line before the socket closes.
	tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn hub() {
	let path = loop_bus::socket_path();
	if UnixStream::connect(&path).await.is_ok() {
		eprintln!("loop-bus: a hub is already running at {}", path.display());
		return;
	}
	if let Some(dir) = path.parent() {
		std::fs::create_dir_all(dir).expect("create the bus directory");
	}
	let _ = std::fs::remove_file(&path);
	let listener = UnixListener::bind(&path).expect("bind the bus socket");
	eprintln!("loop-bus: listening at {}", path.display());

	let (hub, _) = broadcast::channel::<(u64, String)>(1024);
	let mut next_id = 0u64;
	let shutdown = tokio::signal::ctrl_c();
	tokio::pin!(shutdown);
	let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
	loop {
		tokio::select! {
			_ = &mut shutdown => break,
			_ = term.recv() => break,
			accepted = listener.accept() => {
				let Ok((stream, _)) = accepted else { continue };
				next_id += 1;
				tokio::spawn(serve(next_id, stream, hub.clone()));
			}
		}
	}
	let _ = std::fs::remove_file(&path);
}

async fn serve(id: u64, stream: UnixStream, hub: broadcast::Sender<(u64, String)>) {
	let mut from_hub = hub.subscribe();
	let (read, mut write) = stream.into_split();
	let mut lines = BufReader::new(read).lines();
	loop {
		tokio::select! {
			line = lines.next_line() => {
				let Ok(Some(line)) = line else { return };
				let _ = hub.send((id, line));
			}
			msg = from_hub.recv() => match msg {
				Ok((sender, line)) if sender != id => {
					let write_line = async {
						write.write_all(line.as_bytes()).await?;
						write.write_all(b"\n").await
					};
					// A service that stops reading is dropped rather than stalling the others.
					match tokio::time::timeout(Duration::from_secs(2), write_line).await {
						Ok(Ok(())) => {}
						_ => return,
					}
				}
				Ok(_) => {}
				Err(broadcast::error::RecvError::Lagged(n)) => eprintln!("loop-bus: service {id} missed {n} events"),
				Err(broadcast::error::RecvError::Closed) => return,
			}
		}
	}
}
