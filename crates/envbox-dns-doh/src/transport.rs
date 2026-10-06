use crate::{executor::Executor, trust::Snapshot, Budget, Error};
use bytes::Bytes;
use futures_util::FutureExt;
use http::{header, Request, Uri};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use rustls::{pki_types::ServerName, ClientConfig};
use std::{
    net::{IpAddr, SocketAddr},
    panic::AssertUnwindSafe,
    sync::Arc,
};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

fn endpoint(url: &str, ip: &str) -> Result<(Uri, SocketAddr, ServerName<'static>), Error> {
    if url.len() > 2047 || url.contains('#') || url.bytes().any(|c| c <= b' ' || c == 127) {
        return Err(Error::Argument);
    }
    let uri: Uri = url.parse().map_err(|_| Error::Argument)?;
    if uri.scheme_str() != Some("https") {
        return Err(Error::Argument);
    }
    let authority = uri.authority().ok_or(Error::Argument)?;
    if authority.as_str().contains('@') {
        return Err(Error::Argument);
    }
    let port_suffix = &authority.as_str()[authority.host().len()..];
    let port = if port_suffix.is_empty() {
        443
    } else {
        port_suffix
            .strip_prefix(':')
            .ok_or(Error::Argument)?
            .parse::<u16>()
            .map_err(|_| Error::Argument)?
    };
    if port == 0 {
        return Err(Error::Argument);
    }
    let host = authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let name = ServerName::try_from(host.to_owned()).map_err(|_| Error::Argument)?;
    let address: IpAddr = ip.parse().map_err(|_| Error::Argument)?;
    Ok((uri, SocketAddr::new(address, port), name))
}

pub(crate) fn query(
    url: &str,
    ip: &str,
    packet: &[u8],
    budget: Budget,
    supplied: Option<Snapshot>,
) -> Result<Vec<u8>, Error> {
    budget.check()?;
    let verification_scope = crate::verification_scope::VerificationScope::enter(budget)?;
    if !(12..=65535).contains(&packet.len()) {
        return Err(Error::Argument);
    }
    let (uri, endpoint, name) = endpoint(url, ip)?;
    let snapshot = match supplied {
        Some(snapshot) => snapshot,
        None => Snapshot::load(budget)?,
    };
    let verifier = snapshot.verifier(budget)?;
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|_| Error::Tls)?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    budget.check()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Error::Network)?;
    runtime.block_on(async {
        let executor = Executor::default();
        let result = {
            let work = AssertUnwindSafe(exchange(
                uri,
                endpoint,
                name,
                packet,
                Arc::new(config),
                executor.clone(),
                budget,
            ))
            .catch_unwind();
            tokio::pin!(work);
            tokio::select! {
                result = &mut work => result.unwrap_or(Err(Error::Panic)),
                error = budget.stopped() => Err(error),
            }
        }; // Drop the exchange future and its sender/stream before cleanup.
        executor.close().await;
        if let Some(error) = verification_scope.stop_reason() {
            return Err(error);
        }
        budget.check()?;
        if executor.exhausted() {
            return Err(Error::Http);
        }
        result
    })
}

async fn exchange(
    uri: Uri,
    endpoint: SocketAddr,
    name: ServerName<'static>,
    packet: &[u8],
    config: Arc<ClientConfig>,
    executor: Executor,
    budget: Budget,
) -> Result<Vec<u8>, Error> {
    budget.check()?;
    // SocketAddr implements Tokio's immediate address path: no ToSocketAddrs
    // hostname, blocking resolver, resolver threadpool, or high-level Client.
    let tcp = TcpStream::connect(endpoint)
        .await
        .map_err(|_| Error::Network)?;
    budget.check()?;
    let tls = TlsConnector::from(config)
        .connect(name, tcp)
        .await
        .map_err(|error| {
            error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<rustls::Error>())
                .cloned()
                .map(Error::from)
                .unwrap_or(Error::Tls)
        })?;
    // TLS verification is synchronous inside this await. Check immediately
    // afterwards before queuing any HTTP application data.
    budget.check()?;
    let http2 = match tls.get_ref().1.alpn_protocol() {
        Some(b"h2") => true,
        Some(b"http/1.1") | None => false,
        _ => return Err(Error::Tls),
    };
    let target = if http2 {
        uri.clone()
    } else {
        uri.path_and_query()
            .map(|value| value.as_str())
            .unwrap_or("/")
            .parse()
            .map_err(|_| Error::Argument)?
    };
    let mut request = Request::builder()
        .method("POST")
        .uri(target)
        .header(header::CONTENT_TYPE, "application/dns-message")
        .header(header::ACCEPT, "application/dns-message");
    // HTTP/2 derives :authority from the absolute URI. Sending a second Host
    // field is rejected by some DoH servers with RST_STREAM(PROTOCOL_ERROR).
    if !http2 {
        request = request.header(
            header::HOST,
            uri.authority().ok_or(Error::Argument)?.as_str(),
        );
    }
    let request = request
        .body(Full::new(Bytes::copy_from_slice(packet)))
        .map_err(|_| Error::Argument)?;
    if http2 {
        let mut builder = hyper::client::conn::http2::Builder::new(executor.clone());
        builder
            .max_header_list_size(32768)
            .initial_stream_window_size(65535);
        let (mut sender, connection) = builder
            .handshake(TokioIo::new(tls))
            .await
            .map_err(|_| Error::Http)?;
        budget.check()?;
        executor.spawn(async move {
            let _ = connection.await;
        });
        budget.check()?;
        let response = sender.send_request(request).await.map_err(http_error)?;
        budget.check()?;
        let result = read_response(response, budget).await;
        drop(sender);
        result
    } else {
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_headers(64)
            .max_buf_size(32768)
            .handshake(TokioIo::new(tls))
            .await
            .map_err(|_| Error::Http)?;
        budget.check()?;
        executor.spawn(async move {
            let _ = connection.await;
        });
        budget.check()?;
        let response = sender.send_request(request).await.map_err(http_error)?;
        budget.check()?;
        let result = read_response(response, budget).await;
        drop(sender);
        result
    }
}

fn http_error(_error: hyper::Error) -> Error {
    #[cfg(feature = "fixture-trust")]
    eprintln!("fixture HTTP error: {_error:?}");
    Error::Http
}

async fn read_response(
    response: http::Response<Incoming>,
    budget: Budget,
) -> Result<Vec<u8>, Error> {
    if !response.status().is_success() {
        return Err(Error::HttpStatus);
    }
    if response.headers().len() > 64
        || response
            .headers()
            .iter()
            .map(|(name, value)| name.as_str().len() + value.len())
            .sum::<usize>()
            > 32768
    {
        return Err(Error::Http);
    }
    let mut encodings = response.headers().get_all(header::CONTENT_ENCODING).iter();
    if let Some(value) = encodings.next() {
        if encodings.next().is_some()
            || !value
                .to_str()
                .map_err(|_| Error::ContentEncoding)?
                .trim()
                .eq_ignore_ascii_case("identity")
        {
            return Err(Error::ContentEncoding);
        }
    }
    let mut media = response.headers().get_all(header::CONTENT_TYPE).iter();
    let value = media
        .next()
        .ok_or(Error::MediaType)?
        .to_str()
        .map_err(|_| Error::MediaType)?;
    let parsed: mime::Mime = value.parse().map_err(|_| Error::MediaType)?;
    if media.next().is_some()
        || parsed.type_() != mime::APPLICATION
        || parsed.subtype().as_str() != "dns-message"
    {
        return Err(Error::MediaType);
    }
    if let Some(length) = response.headers().get(header::CONTENT_LENGTH) {
        if length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .is_none_or(|value| value > 65535)
        {
            return Err(Error::BodyLimit);
        }
    }
    budget.check()?;
    read_body(response.into_body(), budget).await
}

async fn read_body(mut body: Incoming, budget: Budget) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        budget.check()?;
        let frame = frame.map_err(http_error)?;
        if let Some(data) = frame.data_ref() {
            if bytes.len() + data.len() > 65535 {
                return Err(Error::BodyLimit);
            }
            bytes.extend_from_slice(data);
        } else if let Some(trailers) = frame.trailers_ref() {
            if trailers.len() > 64
                || trailers
                    .iter()
                    .map(|(name, value)| name.as_str().len() + value.len())
                    .sum::<usize>()
                    > 32768
            {
                return Err(Error::Http);
            }
        }
    }
    if bytes.len() < 12 {
        return Err(Error::BodyLimit);
    }
    budget.check()?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_budget_at_exchange_entry_opens_no_connection() {
        // The runtime is already constructed, isolating the boundary after a
        // potentially slow runtime setup and before the first network action.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = listener.local_addr().unwrap();
        let config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(exchange(
            "https://fixture.test/dns-query".parse().unwrap(),
            endpoint,
            ServerName::try_from("fixture.test").unwrap(),
            &[0u8; 12],
            Arc::new(config),
            Executor::default(),
            Budget::until(0),
        ));
        assert!(matches!(result, Err(Error::Deadline)));
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "an expired exchange opened a connection before checking its budget"
        );
    }
}
