use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:11212").await?;

    loop {
        let (socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let (read_half, write_half) = socket.into_split();
            let mut reader = BufReader::new(read_half);
            let mut writer = write_half;
            let mut line = String::new();

            loop {
                line.clear();
                let bytes = reader.read_line(&mut line).await.unwrap();
                if bytes == 0 {
                    break;
                }

                let v: Vec<&str> = line.split(char::is_whitespace).collect();
                match v[0] {
                    "get" => {
                        println!("command get got {}", v[1]);
                        let value = "response";
                        writer
                            .write_all(
                                format!("VALUE {} 0 {}\r\n{}\r\nEND\r\n", v[1], value.len(), value)
                                    .as_bytes(),
                            )
                            .await
                            .unwrap();
                    }
                    "version" => {
                        writer.write_all(b"VERSION 0.0.1\r\n").await.unwrap();
                    }
                    _ => {
                        println!("unknown command {}", line);
                        writer.write_all(b"ERROR\r\n").await.unwrap();
                    }
                }
            }
        });
    }
    Ok(())
}
