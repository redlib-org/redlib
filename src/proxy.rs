use base64::engine::general_purpose;
use base64::Engine;
use hyper::client::HttpConnector;
use hyper::service::Service;
use hyper::Uri;
use log::debug;
use std::env;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::net::TcpStream;
use tokio_socks::tcp::Socks5Stream;

type BoxError = Box<dyn Error + Send + Sync>;
type BoxFuture<T> = Pin<Box<dyn Future<Output = Result<T, BoxError>> + Send>>;
type Credentials = (String, String);

#[derive(Clone)]
pub enum ProxyConnector {
    NoProxy(HttpConnector),
    Socks(String),
    Http(String),
}

#[derive(Debug)]
pub struct ProxyError(String);

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Proxy error: {}", self.0)
    }
}

impl Error for ProxyError {}

impl Service<Uri> for ProxyConnector {
    type Response = TcpStream;
    type Error = BoxError;
    type Future = BoxFuture<Self::Response>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self {
            ProxyConnector::NoProxy(connector) => connector.poll_ready(cx).map_err(Into::into),
            _ => Poll::Ready(Ok(())),
        }
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            match this {
                ProxyConnector::NoProxy(mut connector) => {
                    connector.call(uri).await.map_err(Into::into)
                }
                ProxyConnector::Socks(proxy_addr) => handle_socks_connection(&proxy_addr, &uri).await,
                ProxyConnector::Http(proxy_addr) => handle_http_connection(&proxy_addr, &uri).await,
            }
        })
    }
}

impl ProxyConnector {
    pub fn new() -> Self {
        if let Ok(socks_proxy) = env::var("SOCKS_PROXY") {
            debug!("Using SOCKS proxy: {}", socks_proxy);
            return ProxyConnector::Socks(socks_proxy);
        }

        if let Ok(http_proxy) = env::var("HTTP_PROXY").or_else(|_| env::var("HTTPS_PROXY")) {
            debug!("Using HTTP proxy: {}", http_proxy);
            return ProxyConnector::Http(http_proxy);
        }

        let mut connector = HttpConnector::new();
        connector.enforce_http(false);
        ProxyConnector::NoProxy(connector)
    }
}

async fn handle_socks_connection(proxy_addr: &str, uri: &Uri) -> Result<TcpStream, BoxError> {
    let (host, port, credentials) = parse_proxy_addr(proxy_addr)?;
    let target_addr = get_target_addr(uri)?;

    let stream = match credentials {
        Some((username, password)) => {
            Socks5Stream::connect_with_password((host.as_str(), port), target_addr, &username, &password).await
        }
        None => Socks5Stream::connect((host.as_str(), port), target_addr).await,
    }?;

    Ok(stream.into_inner())
}

async fn handle_http_connection(proxy_addr: &str, uri: &Uri) -> Result<TcpStream, BoxError> {
    let (host, port, credentials) = parse_proxy_addr(proxy_addr)?;
    let proxy_stream = TcpStream::connect((host.as_str(), port)).await?;
    let target_addr = get_target_addr(uri)?;

    let connect_req = build_connect_request(&target_addr, credentials)?;
    write_and_verify_connection(&proxy_stream, &connect_req).await?;

    Ok(proxy_stream)
}

fn build_connect_request(target_addr: &str, credentials: Option<Credentials>) -> Result<String, BoxError> {
    let mut req = format!(
        "CONNECT {target_addr} HTTP/1.1\r\n\
         Host: {target_addr}\r\n\
         Connection: keep-alive\r\n"
    );

    if let Some((username, password)) = credentials {
        let auth = general_purpose::STANDARD.encode(format!("{}:{}", username, password));
        req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", auth));
    }

    req.push_str("\r\n");
    Ok(req)
}

async fn write_and_verify_connection(proxy_stream: &TcpStream, connect_req: &str) -> Result<(), BoxError> {
    proxy_stream.writable().await?;
    proxy_stream.try_write(connect_req.as_bytes())?;

    let mut response = [0u8; 1024];
    proxy_stream.readable().await?;
    let n = proxy_stream.try_read(&mut response)?;

    let response = String::from_utf8_lossy(&response[..n]);
    if !response.starts_with("HTTP/1.1 200") {
        return Err(Box::new(ProxyError(format!("Proxy CONNECT failed: {}", response))));
    }

    Ok(())
}

fn parse_proxy_addr(addr: &str) -> Result<(String, u16, Option<Credentials>), BoxError> {
    let uri: Uri = addr.parse()?;
    let host = uri.host().ok_or("Missing proxy host")?.to_string();
    let port = uri.port_u16().unwrap_or_else(|| {
        if uri.scheme_str() == Some("https") { 443 } else { 80 }
    });

    let credentials = extract_credentials(uri.authority())?;
    Ok((host, port, credentials))
}

fn extract_credentials(authority: Option<&hyper::http::uri::Authority>) -> Result<Option<Credentials>, BoxError> {
    let Some(authority) = authority else {
        return Ok(None);
    };

    let Some(credentials) = authority.as_str().split('@').next() else {
        return Ok(None);
    };

    if credentials == authority.as_str() {
        return Ok(None);
    }

    let creds: Vec<&str> = credentials.split(':').collect();
    if creds.len() == 2 {
        Ok(Some((creds[0].to_string(), creds[1].to_string())))
    } else {
        Ok(None)
    }
}

fn get_target_addr(uri: &Uri) -> Result<String, BoxError> {
    let host = uri.host().ok_or("Missing target host")?;
    let port = uri.port_u16().unwrap_or_else(|| {
        if uri.scheme_str() == Some("https") { 443 } else { 80 }
    });
    Ok(format!("{}:{}", host, port))
}