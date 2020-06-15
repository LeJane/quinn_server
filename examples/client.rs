use anyhow::{anyhow, Result};
use byteorder::{LittleEndian, WriteBytesExt};
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

    let mut client_build = client_config.build();
    let client_trans = std::sync::Arc::get_mut(&mut client_build.transport).unwrap();
    client_trans
        .max_idle_timeout(Some(std::time::Duration::new(60, 0)))
        .unwrap();
    client_trans.keep_alive_interval(Some(std::time::Duration::new(120, 0)));

    let start = Instant::now();

    enpoint.default_client_config(client_build);
    let (endpoint, mut _incoming) = enpoint.bind(&"[::]:0".parse().unwrap())?;

    let socket = std::net::UdpSocket::bind("[::]:0").unwrap();
    let addr = socket.local_addr().unwrap();
    eprintln!("rebinding to {}", addr);
    endpoint.rebind(socket).expect("rebind failed");

    let quinn::NewConnection {
        connection,
        bi_streams,
        ..
    } = endpoint
        .connect(&remote, "localhost")
        .unwrap()
        .await
        .expect("connect");

    println!("connected at {:?}", start.elapsed());
    tokio::spawn(async move {
        if let Err(e) = handle_connection(bi_streams).await {
            // connection.remote_address()
            error!("connection failed: {reason}", reason = e.to_string());
        }
    });

    loop {
        let mut input = String::new();
        match sysio::stdin().read_line(&mut input) {
            Ok(_n) => {
                println!("input:{}", input);

                let (mut s, recv) = connection
                    .open_bi()
                    .await
                    .map_err(|e| {
                        error!("failed open stream:{}", e);
                    })
                    .unwrap();log_syntax!()

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
                    .map_err(|e| anyhow!("failed to read response stream: {}", e))
                    .unwrap();

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

async fn handle_connection(mut bi_streams: quinn::IncomingBiStreams) -> Result<()> {
    info!("established");
    //new request
    while let Some(stream) = bi_streams.next().await {
        let stream = match stream {
            Err(quinn::ConnectionError::ApplicationClosed { .. }) => {
                info!("connection closed");
                return Ok(());
            }
            Err(e) => {
                info!("connection errp");
                return Err(e.into());
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

//handle request
async fn handle_request((mut _send, recv): (quinn::SendStream, quinn::RecvStream)) -> Result<()> {
    let req = recv
        .read_to_end(64 * 1024)
        .await
        .map_err(|e| anyhow!("failed reading request:{}", e))?;
    let mut escaped = String::new();

    for &x in &req[..] {
        let part = ascii::escape_default(x).collect::<Vec<_>>();
        escaped.push_str(str::from_utf8(&part).unwrap());
    }

    println!("接收content：{:?}", escaped);

    info!(content=%escaped);

    info!("complete");
    Ok(())
}
