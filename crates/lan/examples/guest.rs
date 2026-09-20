//! A guest at a LAN room, from the terminal: joins, sits, says hello, and
//! prints what the room says until the game starts or you press Ctrl-C.
//!
//!     cargo run -p lan --example guest -- 192.168.1.5:8200 bob [password]
//!
//! For trying a hosted room without a second machine, or a second machine
//! without a second modlobby.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

#[tokio::main]
async fn main() {
	let mut args = std::env::args().skip(1);
	let addr = args.next().unwrap_or_else(|| "127.0.0.1:8200".into());
	let name = args.next().unwrap_or_else(|| "guest".into());
	let password = args.next().unwrap_or_else(|| "empty".into());
	let stream = TcpStream::connect(&addr).await.expect("connect");
	let (read, mut write) = stream.into_split();
	let mut lines = BufReader::new(read).lines();
	write
		.write_all(format!("LOGIN {name} * 0 * modlobby-guest:0\tx\tb sp\n").as_bytes())
		.await
		.unwrap();
	while let Ok(Some(line)) = lines.next_line().await {
		println!("< {line}");
		match line.split(' ').next().unwrap_or("") {
			"LOGININFOEND" => {
				let out = format!("JOINBATTLE 1 {password} 4242\n");
				write.write_all(out.as_bytes()).await.unwrap();
			}
			"REQUESTBATTLESTATUS" => {
				// A seat on team 1, ally 1, synced, and a word.
				let bits = spring_protocol::MyBattleStatus::player(spring_protocol::Sync::Synced, 1, 1).bits();
				write
					.write_all(format!("MYBATTLESTATUS {bits} 255\nSAYBATTLE hello from the terminal\n").as_bytes())
					.await
					.unwrap();
			}
			"DENIED" | "JOINBATTLEFAILED" => break,
			_ => {}
		}
	}
}
