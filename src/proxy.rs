use hyper::client::HttpConnector;
use hyper::service::Service;
use hyper::Uri;
use std::env;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::net::TcpStream;
use tokio_socks::tcp::Socks5Stream;
use log::debug;
use std::fmt;
use base64::Engine;
use base64::engine::general_purpose;

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
    type Error = Box<dyn Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self {
            ProxyConnector::NoProxy(connector) => connector.poll_ready(cx).map_err(Into::into),
            _ => Poll::Ready(Ok(()))
        }
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            match this {
                ProxyConnector::NoProxy(mut connector) => {
                    let stream = connector.call(uri).await.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;
                    Ok(stream)
                }
                ProxyConnector::Socks(proxy_addr) => {
                    let (host, port, credentials) = parse_proxy_addr(&proxy_addr)?;
                    let target_addr = get_target_addr(&uri)?;

                    let stream = match credentials {
                        Some((username, password)) => {
                            Socks5Stream::connect_with_password(
                                (host.as_str(), port),
                                target_addr,
                                &username,
                                &password
                            ).await
                        },
                        None => {
                            Socks5Stream::connect(
                                (host.as_str(), port),
                                target_addr
                            ).await
                        }
                    }.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;

                    Ok(stream.into_inner())

                }
                ProxyConnector::Http(proxy_addr) => {
                    let (host, port, credentials) = parse_proxy_addr(&proxy_addr)?;
                    let proxy_stream = TcpStream::connect((host.as_str(), port)).await.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;

                    let target_addr = get_target_addr(&uri)?;
                    let mut connect_req = format!(
                        "CONNECT {target_addr} HTTP/1.1\r\n\
                         Host: {target_addr}\r\n\
                         Connection: keep-alive\r\n"
                    );

                    if let Some((username, password)) = credentials {
                        let auth = general_purpose::STANDARD.encode(format!("{}:{}", username, password));
                        connect_req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", auth));
                    }

                    connect_req.push_str("\r\n");
                    proxy_stream.writable().await.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;
                    proxy_stream.try_write(connect_req.as_bytes()).map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;

                    let mut response = [0u8; 1024];
                    proxy_stream.readable().await.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;
                    let n = proxy_stream.try_read(&mut response).map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;

                    let response = String::from_utf8_lossy(&response[..n]);
                    if !response.starts_with("HTTP/1.1 200") {
                        return Err(Box::new(ProxyError(format!("Proxy CONNECT failed: {}", response))) as Box<dyn Error + Send + Sync>);
                    }

                    Ok(proxy_stream)
                }
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

fn parse_proxy_addr(addr: &str) -> Result<(String, u16, Option<(String, String)>), Box<dyn Error + Send + Sync>> {
    let uri: Uri = addr.parse()?;
    let host = uri.host().ok_or("Missing proxy host")?.to_string();
    let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("https") { 443 } else { 80 });

    let credentials = if let Some(authority) = uri.authority() {
        if let Some(credentials) = authority.as_str().split('@').next() {
            if credentials != authority.as_str() {
                let creds: Vec<&str> = credentials.split(':').collect();
                if creds.len() == 2 {
                    Some((creds[0].to_string(), creds[1].to_string()))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok((host, port, credentials))
}


fn get_target_addr(uri: &Uri) -> Result<String, Box<dyn Error + Send + Sync>> {
    let host = uri.host().ok_or("Missing target host")?;
    let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("https") { 443 } else { 80 });
    Ok(format!("{}:{}", host, port))
}