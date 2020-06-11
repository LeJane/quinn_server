
use anyhow::{anyhow, Result};
use byteorder::{LittleEndian,  WriteBytesExt};
use futures::{StreamExt, TryFutureExt};
use std::{
    ascii, fs,
    io::{self as sysio},
    net::ToSocketAddrs,
    str,
    time::Instant,
};
use tracing::{error, info, info_span};
use tracing_futures::Instrument as _;

#[tokio::main]
async fn main() -> Result<()> {
    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .finish(),
    )
    .unwrap();

    let remote = ("127.0.0.1", 4332)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("couldn't resolve to an address"))?;

    
    let mut enpoint = quinn::Endpoint::builder();
    let mut client_config = quinn::ClientConfigBuilder::default();

    
    let dirs = directories::ProjectDirs::from("org", "client", "myquinn").unwrap();
    match fs::read(dirs.data_local_dir().join("cert.der")) {
        Ok(cert) => {
            client_config.add_certificate_authority(quinn::Certificate::from_der(&cert)?)?;
        }
        Err(ref e) if e.kind() == sysio::ErrorKind::NotFound => {
            info!("local server certificate not found");
        }
        Err(e) => {
            error!("failed to open local server certificate:{}", e);
        }
    }

    let start = Instant::now();

    enpoint.default_client_config(client_config.build());
    let (endpoint, mut incoming) = enpoint.bind(&"[::]:0".parse().unwrap())?;
    println!("connected at {:?}", start.elapsed());
    tokio::spawn(async move {
        println!("adfasdf");
        while let Some(connecting) = incoming.next().await {
            println!("server connection incoming");
            tokio::spawn(handle_connection(connecting).unwrap_or_else(move |e| {
                error!("connection failed: {reason}", reason = e.to_string());
            }));
        }
    });

    loop {
        let mut input = String::new();
        match sysio::stdin().read_line(&mut input) {
            Ok(_n) => {
                println!("input:{}", input);

                let conn = endpoint
                    .connect(&remote, "localhost")
                    .unwrap()
                    .await
                    .expect("connect")
                    .connection;
                let (mut s, recv) = conn.open_bi().await.unwrap();

                let mut request = vec![];
                request.write_u16::<LittleEndian>(1001).unwrap();
                request.write_u8(1).unwrap();
                request
                    .write_u16::<LittleEndian>(input.as_bytes().len() as u16)
                    .unwrap();
                // request.write_u64::<LittleEndian>().unwrap();

                request.extend_from_slice(input.as_bytes());

                s.write_all(input.as_bytes()).await.expect("send error.");
                s.finish().await.unwrap();

                let resp: Vec<u8> = recv
                    .read_to_end(usize::max_value())
                    .await
                    .map_err(|e| anyhow!("failed to read response stream: {}", e))?;

                println!(
                    "response received in {:?}",
                    std::str::from_utf8(&resp).unwrap()
                );
                // conn.close(0u32.into(), b"");
            }
            Err(error) => println!("error: {}", error),
        }
    }
}

async fn handle_connection(conn: quinn::Connecting) -> Result<()> {
    let quinn::NewConnection {
        // connection,
        mut bi_streams,
        ..
    } = conn.await?;

    async {
        info!("established");
        //new request
        while let Some(stream) = bi_streams.next().await {
            let stream = match stream {
                Err(quinn::ConnectionError::ApplicationClosed { .. }) => {
                    info!("connection closed");
                    return Ok(());
                }
                Err(e) => {
                    return Err(e);
                }
                Ok(s) => s,
            };

            tokio::spawn(
                handle_request(stream)
                    .unwrap_or_else(move |e| error!("failed:{reason}", reason = e.to_string()))
                    .instrument(info_span!("request")),
            );
        }
        Ok(())
    }
    .await?;
    Ok(())
}

//handle request
async fn handle_request((mut send, recv): (quinn::SendStream, quinn::RecvStream)) -> Result<()> {
    println!("request start.");
    let req = recv
        .read_to_end(64 * 1024)
        .await
        .map_err(|e| anyhow!("failed reading request:{}", e))?;
    let mut escaped = String::new();

    for &x in &req[..] {
        let part = ascii::escape_default(x).collect::<Vec<_>>();
        escaped.push_str(str::from_utf8(&part).unwrap());
    }

    println!("content：{:?}", escaped);

    info!(content=%escaped);

    //excute request

    let resp = "success...";

    send.write_all(&resp.as_bytes())
        .await
        .map_err(|e| anyhow!("failed to send response:{}", e))?;

    send.finish()
        .await
        .map_err(|e| anyhow!("failed to shutdown stream:{}", e))?;

    info!("complete");
    Ok(())
}
