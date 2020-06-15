use anyhow::{anyhow, bail, Context, Result};
use futures::{StreamExt, TryFutureExt};
use quinn::{CertificateChain, PrivateKey};
use std::sync::Arc;
use std::{fs, io, net::IpAddr, net::Ipv4Addr, net::SocketAddr, str};
use tracing::{error, info, info_span};
use tracing_futures::Instrument as _;
use tracing_subscriber::filter::LevelFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive(LevelFilter::INFO.into()),
            )
            .finish(),
    )?;

    let (_cert, cert_chain, key) = build_certs().expect("failed build certificate.");

    let mut server = quinn::ServerConfigBuilder::default().build();
    let socket_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4332);
    server.certificate(cert_chain, key)?;
    let mut endpoint = quinn::Endpoint::builder();
    endpoint.listen(server);

    let (endpoint, mut incoming) = endpoint.bind(&socket_addr).expect("bind failed");
    info!("server listening on {:?}", endpoint.local_addr());

    // let addr = endpoint_arc.local_addr().unwrap();

    while let Some(connecting) = incoming.next().await {
        info!("connection incoming");

        let connection = connecting.await?;

        let _s = connection.connection.open_uni().await?;

        let bi_streams = connection.bi_streams;

        write_to_peer_connection(&connection.connection, b"asdfasdfas".to_vec()).await?;

        tokio::spawn(
            handle_conection(connection.connection, bi_streams).unwrap_or_else(move |e| {
                error!("connection failed: {reason}", reason = e.to_string());
            }),
        );
    }

    println!("Hello, world!");
    Ok(())
}
#[allow(unused)]
async fn handle_conection(
    connection: quinn::Connection,
    mut bi_streams: quinn::IncomingBiStreams,
) -> Result<()> {
    println!("connection start.");

    let span = info_span!(
        "connection",
        remote = %connection.remote_address(),
        protocol = %connection
            .authentication_data()
            .protocol
            .map_or_else(|| "<none>".into(), |x| String::from_utf8_lossy(&x).into_owned())
    );

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
    .instrument(span)
    .await?;
    Ok(())
}

//处理请求
#[allow(unused)]
async fn handle_request(
    // conn: quinn::Connection,
    (mut send, recv): (quinn::SendStream, quinn::RecvStream),
) -> Result<()> {
    println!("request start.");

    let req = recv
        .read_to_end(64 * 1024)
        .await
        .map_err(|e| anyhow!("failed reading request:{}", e))?;

    println!("内容：{:?}", str::from_utf8(&req).unwrap());

    //excute request

    send.write_all(&req)
        .await
        .map_err(|e| anyhow!("failed to send response:{}", e))?;

    send.finish()
        .await
        .map_err(|e| anyhow!("failed to shutdown stream:{}", e))?;

    Ok(())
}

//构建加密验证
fn build_certs() -> Result<(Vec<u8>, CertificateChain, PrivateKey)> {
    let dirs = directories::ProjectDirs::from("org", "server", "myquinn").unwrap();
    let path = dirs.data_local_dir();
    println!("local path:{:?}", &path);
    let cert_path = path.join("cert.der");
    let key_path = path.join("key.der");
    let (cert_vec, key) = match fs::read(&cert_path).and_then(|x| Ok((x, fs::read(&key_path)?))) {
        Ok(x) => x,
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => {
            //创建加密key
            info!("generating self-signed certificate");
            let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let key = cert.serialize_private_key_der();
            let cert = cert.serialize_der().unwrap();
            fs::create_dir_all(&path).context("failed to create certificate directory.")?;
            fs::write(&cert_path, &cert).context("failed to write certificate")?;
            fs::write(&key_path, &key).context("failed to write privite key.")?;
            (cert, key)
        }
        Err(e) => {
            bail!("failed to read certificate:{}", e);
        }
    };

    let key = quinn::PrivateKey::from_der(&key)?;
    let cert = quinn::Certificate::from_der(&cert_vec)?;

    println!("success to ger certificate.");
    Ok((
        cert_vec,
        quinn::CertificateChain::from_certs(vec![cert]),
        key,
    ))
}

fn configure_client_connector() -> quinn::ClientConfig {
    let mut peer_cfg_builder = quinn::ClientConfigBuilder::default();
    //获取加密证书
    let dirs = directories::ProjectDirs::from("org", "client", "myquinn").unwrap();
    match fs::read(dirs.data_local_dir().join("cert.der")) {
        Ok(cert) => {
            peer_cfg_builder
                .add_certificate_authority(quinn::Certificate::from_der(&cert).unwrap())
                .unwrap();
        }
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => {
            info!("local server certificate not found");
        }
        Err(e) => {
            error!("failed to open local server certificate:{}", e);
        }
    };

    let peer_cfg = peer_cfg_builder.build();

    peer_cfg
}
#[allow(unused)]
async fn send_data_to(
    endpoint: Arc<quinn::Endpoint>,
    addr: SocketAddr,
    data: Vec<u8>,
) -> Result<()> {
    let client_cfg = configure_client_connector();
    println!("addr:{:?}", addr);
    let connecting = endpoint
        .connect_with(client_cfg, &addr, "Test")
        .map_err(|e| panic!("Connection failed: {}", e))
        .expect("failed open connting.")
        .await
        .unwrap();

    let new_conn = connecting.connection;
    println!("[client] connected: addr={}", new_conn.remote_address());

    write_to_peer_connection(&new_conn, data).await?;
    Ok(())
}

#[allow(unused)]
async fn write_to_peer_connection(conn: &quinn::Connection, data: Vec<u8>) -> Result<()> {
    let (mut sender, _) = conn.open_bi().await.expect("failed open bi stream");

    sender
        .write_all(&data)
        .await
        .map_err(|e| anyhow!("failed to send response:{}", e))?;

    sender
        .finish()
        .await
        .map_err(|e| anyhow!("failed to shutdown stream:{}", e))?;

    Ok(())
}
